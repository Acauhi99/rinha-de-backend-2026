pub struct Index {
    backing: Box<[u8]>,
    pub num_vectors: u32,
    pub dim: u32,
    pub k_clusters: u32,
    pub scale: u32,
    centroids_off: usize,
    vectors_off: usize,
    labels_off: usize,
    orig_ids_off: usize,
    cluster_counts_off: usize,
    cluster_byte_offsets_off: usize,
    bbox_min_off: usize,
    bbox_max_off: usize,
}

impl Index {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let data = std::fs::read(path)?;
        let data = data.into_boxed_slice();

        if data.len() < 64 || &data[0..4] != b"RVIF" {
            return Err("invalid index file: bad magic".into());
        }

        let version = u32::from_le_bytes(data[4..8].try_into()?);
        if version != 2 {
            return Err("unsupported index version (expected v2)".into());
        }

        let num_vectors = u32::from_le_bytes(data[8..12].try_into()?);
        let dim = u32::from_le_bytes(data[12..16].try_into()?);
        let k_clusters = u32::from_le_bytes(data[16..20].try_into()?);
        let scale = u32::from_le_bytes(data[20..24].try_into()?);

        let cs = (k_clusters * dim * 4) as usize;
        let ls = num_vectors as usize;
        let os = (num_vectors * 4) as usize;
        let ccs = (k_clusters * 4) as usize; // cluster_counts
        let cbs = ((k_clusters + 1) * 4) as usize; // cluster_byte_offsets
        let bs = (k_clusters * dim * 2) as usize;

        let centroids_off = 64usize;
        let vectors_off = centroids_off + cs;

        let total = data.len();
        let bbox_max_off = total - bs;
        let bbox_min_off = bbox_max_off - bs;
        let cluster_byte_offsets_off = bbox_min_off - cbs;
        let _cluster_of_off = cluster_byte_offsets_off - os;
        let cluster_counts_off = _cluster_of_off - ccs;
        let orig_ids_off = cluster_counts_off - os;
        let labels_off = orig_ids_off - ls;

        let expected = bbox_max_off + bs;
        if data.len() < expected {
            return Err("index file truncated".into());
        }

        Ok(Index {
            backing: data,
            num_vectors,
            dim,
            k_clusters,
            scale,
            centroids_off,
            vectors_off,
            labels_off,
            orig_ids_off,
            cluster_counts_off,
            cluster_byte_offsets_off,
            bbox_min_off,
            bbox_max_off,
        })
    }

    fn centroids(&self) -> &[f32] {
        let len = (self.k_clusters * self.dim) as usize;
        unsafe { std::slice::from_raw_parts(self.backing.as_ptr().add(self.centroids_off) as *const f32, len) }
    }

    fn labels(&self) -> &[u8] {
        let len = self.num_vectors as usize;
        &self.backing[self.labels_off..self.labels_off + len]
    }

    fn orig_ids(&self) -> &[u32] {
        let len = self.num_vectors as usize;
        unsafe { std::slice::from_raw_parts(self.backing.as_ptr().add(self.orig_ids_off) as *const u32, len) }
    }

    fn cluster_counts(&self) -> &[u32] {
        let len = self.k_clusters as usize;
        unsafe { std::slice::from_raw_parts(self.backing.as_ptr().add(self.cluster_counts_off) as *const u32, len) }
    }

    fn cluster_byte_offsets(&self) -> &[u32] {
        let len = (self.k_clusters + 1) as usize;
        unsafe { std::slice::from_raw_parts(self.backing.as_ptr().add(self.cluster_byte_offsets_off) as *const u32, len) }
    }

    fn cluster_info(&self, cluster: u32) -> (usize, usize, usize, usize) {
        let byte_offs = self.cluster_byte_offsets();
        let counts = self.cluster_counts();
        let byte_start = self.vectors_off + byte_offs[cluster as usize] as usize;
        let byte_end = self.vectors_off + byte_offs[(cluster + 1) as usize] as usize;
        let mut vec_start = 0usize;
        for k in 0..cluster as usize {
            vec_start += counts[k] as usize;
        }
        let vec_end = vec_start + counts[cluster as usize] as usize;
        (byte_start, byte_end, vec_start, vec_end)
    }

    fn cluster_padded(&self, cluster: u32) -> usize {
        let byte_offs = self.cluster_byte_offsets();
        let start = byte_offs[cluster as usize] as usize;
        let end = byte_offs[(cluster + 1) as usize] as usize;
        (end - start) / (2 * self.dim as usize)
    }

    fn bbox_min_for_cluster(&self, cluster: u32) -> &[i16] {
        let len = self.dim as usize;
        let off = self.bbox_min_off + (cluster as usize) * len * 2;
        unsafe { std::slice::from_raw_parts(self.backing.as_ptr().add(off) as *const i16, len) }
    }

    fn bbox_max_for_cluster(&self, cluster: u32) -> &[i16] {
        let len = self.dim as usize;
        let off = self.bbox_max_off + (cluster as usize) * len * 2;
        unsafe { std::slice::from_raw_parts(self.backing.as_ptr().add(off) as *const i16, len) }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    pub dist: u32,
    pub orig_id: u32,
    pub label: u8,
}

struct FixedHeap<const K: usize> {
    items: [Candidate; K],
    len: usize,
}

impl<const K: usize> FixedHeap<K> {
    fn new() -> Self {
        FixedHeap {
            items: [Candidate { dist: u32::MAX, orig_id: u32::MAX, label: 0 }; K],
            len: 0,
        }
    }

    fn less(a: &Candidate, b: &Candidate) -> bool {
        a.dist < b.dist || (a.dist == b.dist && a.orig_id < b.orig_id)
    }

    fn push(&mut self, cand: Candidate) {
        if self.len < K {
            let mut pos = self.len;
            while pos > 0 && Self::less(&cand, &self.items[pos - 1]) {
                self.items[pos] = self.items[pos - 1];
                pos -= 1;
            }
            self.items[pos] = cand;
            self.len += 1;
        } else if Self::less(&cand, &self.items[K - 1]) {
            let mut pos = K - 1;
            while pos > 0 && Self::less(&cand, &self.items[pos - 1]) {
                self.items[pos] = self.items[pos - 1];
                pos -= 1;
            }
            self.items[pos] = cand;
        }
    }

    fn max_dist(&self) -> u32 {
        if self.len < K { u32::MAX } else { self.items[K - 1].dist }
    }

    fn into_sorted_array(self) -> [Candidate; K] {
        self.items
    }
}

fn scan_cluster_scalar(index: &Index, query: &[i16; 14], cluster: u32, heap: &mut FixedHeap<5>) {
    let dim = index.dim as usize;
    let (_, _, vec_start, vec_end) = index.cluster_info(cluster);
    let padded = index.cluster_padded(cluster);
    let stride = padded * 2;

    let base = unsafe { index.backing.as_ptr().add(index.vectors_off) };
    let byte_offs = index.cluster_byte_offsets();
    let cluster_byte_start = byte_offs[cluster as usize] as usize;

    let labels = index.labels();
    let orig_ids = index.orig_ids();
    let actual = vec_end - vec_start;

    for vi in 0..actual {
        let block = vi / 8;
        let lane = vi % 8;
        let mut sum = 0u32;

        for d in 0..dim {
            let off = cluster_byte_start + d * stride + block * 16 + lane * 2;
            let val = unsafe { *(base.add(off) as *const i16) };
            let q = query[d] as i32;
            let v = val as i32;
            let diff = q - v;
            sum += (diff * diff) as u32;
        }

        let cand = Candidate {
            dist: sum,
            orig_id: orig_ids[vec_start + vi],
            label: labels[vec_start + vi],
        };
        heap.push(cand);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scan_cluster_avx2(index: &Index, query: &[i16; 14], cluster: u32, heap: &mut FixedHeap<5>) {
    use std::arch::x86_64::*;

    let (_byte_start, _, vec_start, vec_end) = index.cluster_info(cluster);
    let actual = vec_end - vec_start;
    let padded = index.cluster_padded(cluster);
    let stride = padded * 2;

    let mut q_broadcast: [__m256i; 14] = [_mm256_setzero_si256(); 14];
    for d in 0..14 {
        q_broadcast[d] = _mm256_set1_epi32(query[d] as i32);
    }

    let byte_offs = index.cluster_byte_offsets();
    let cluster_byte_start = byte_offs[cluster as usize] as usize;
    let base = index.backing.as_ptr().add(index.vectors_off + cluster_byte_start);
    let labels = index.labels();
    let orig_ids = index.orig_ids();

    for block in 0..padded / 8 {
        let mut acc_lo = _mm256_setzero_si256();
        let mut acc_hi = _mm256_setzero_si256();

        for d in 0..14 {
            let col_ptr = base.add(block * 8 * 2 + d * stride) as *const __m128i;
            let col = _mm_loadu_si128(col_ptr);
            let col32 = _mm256_cvtepi16_epi32(col);
            let diff = _mm256_sub_epi32(col32, q_broadcast[d]);
            let sq = _mm256_mullo_epi32(diff, diff);

            let sq_lo128 = _mm256_extracti128_si256::<0>(sq);
            let sq_hi128 = _mm256_extracti128_si256::<1>(sq);
            let sq_lo64 = _mm256_cvtepi32_epi64(sq_lo128);
            let sq_hi64 = _mm256_cvtepi32_epi64(sq_hi128);

            acc_lo = _mm256_add_epi64(acc_lo, sq_lo64);
            acc_hi = _mm256_add_epi64(acc_hi, sq_hi64);
        }

        let mut dists = [0i64; 8];
        _mm256_storeu_si256(dists.as_mut_ptr() as *mut __m256i, acc_lo);
        _mm256_storeu_si256(dists.as_mut_ptr().add(4) as *mut __m256i, acc_hi);

        for lane in 0..8 {
            let vi = block * 8 + lane;
            if vi >= actual { break; }

            let cand = Candidate {
                dist: dists[lane] as u32,
                orig_id: orig_ids[vec_start + vi],
                label: labels[vec_start + vi],
            };
            heap.push(cand);
        }
    }
}

fn scan_cluster(index: &Index, query: &[i16; 14], cluster: u32, heap: &mut FixedHeap<5>) {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { scan_cluster_avx2(index, query, cluster, heap); }
            return;
        }
    }
    scan_cluster_scalar(index, query, cluster, heap);
}

fn min_possible_distance(index: &Index, query: &[i16; 14], cluster: u32) -> u32 {
    let bbox_min = index.bbox_min_for_cluster(cluster);
    let bbox_max = index.bbox_max_for_cluster(cluster);
    let dim = index.dim as usize;
    let mut sum: u32 = 0;

    for d in 0..dim {
        let q = query[d] as i32;
        let lo = bbox_min[d] as i32;
        let hi = bbox_max[d] as i32;

        let diff = if q < lo {
            lo - q
        } else if q > hi {
            q - hi
        } else {
            0
        };

        sum += (diff * diff) as u32;
    }

    sum
}

pub fn search(index: &Index, query: &[i16; 14]) -> [Candidate; 5] {
    let query_f32: [f32; 14] = std::array::from_fn(|i| query[i] as f32 / index.scale as f32);
    let centroids = index.centroids();
    let dim = index.dim as usize;

    let mut best_cluster = 0u32;
    let mut best_centroid_dist = f32::MAX;

    for k in 0..index.k_clusters {
        let c_start = (k as usize) * dim;
        let mut dist = 0.0f32;
        for d in 0..dim {
            let diff = query_f32[d] - centroids[c_start + d];
            dist += diff * diff;
        }
        if dist < best_centroid_dist {
            best_centroid_dist = dist;
            best_cluster = k;
        }
    }

    let mut heap = FixedHeap::<5>::new();
    scan_cluster(index, query, best_cluster, &mut heap);

    for k in 0..index.k_clusters {
        if k == best_cluster { continue; }
        if heap.len < 5 {
            scan_cluster(index, query, k, &mut heap);
            continue;
        }
        let fifth = heap.max_dist();
        let min_dist = min_possible_distance(index, query, k);
        if min_dist < fifth {
            scan_cluster(index, query, k, &mut heap);
        }
    }

    heap.into_sorted_array()
}
