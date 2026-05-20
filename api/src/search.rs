use std::cmp::Ordering;

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
    cluster_offsets_off: usize,
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

        let num_vectors = u32::from_le_bytes(data[8..12].try_into()?);
        let dim = u32::from_le_bytes(data[12..16].try_into()?);
        let k_clusters = u32::from_le_bytes(data[16..20].try_into()?);
        let scale = u32::from_le_bytes(data[20..24].try_into()?);

        let cs = (k_clusters * dim * 4) as usize;
        let vs = (num_vectors * dim * 2) as usize;
        let ls = num_vectors as usize;
        let os = (num_vectors * 4) as usize;
        let cos = ((k_clusters + 1) * 4) as usize;
        let bs = (k_clusters * dim * 2) as usize;

        let centroids_off = 64usize;
        let vectors_off = centroids_off + cs;
        let labels_off = vectors_off + vs;
        let orig_ids_off = labels_off + ls;
        let cluster_offsets_off = orig_ids_off + os + os; // orig_ids + cluster_of
        let bbox_min_off = cluster_offsets_off + cos;
        let bbox_max_off = bbox_min_off + bs;

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
            cluster_offsets_off,
            bbox_min_off,
            bbox_max_off,
        })
    }

    fn centroids(&self) -> &[f32] {
        let len = (self.k_clusters * self.dim) as usize;
        unsafe { std::slice::from_raw_parts(self.backing.as_ptr().add(self.centroids_off) as *const f32, len) }
    }

    fn vectors(&self) -> &[i16] {
        let len = (self.num_vectors * self.dim) as usize;
        unsafe { std::slice::from_raw_parts(self.backing.as_ptr().add(self.vectors_off) as *const i16, len) }
    }

    fn labels(&self) -> &[u8] {
        let len = self.num_vectors as usize;
        &self.backing[self.labels_off..self.labels_off + len]
    }

    fn orig_ids(&self) -> &[u32] {
        let len = self.num_vectors as usize;
        unsafe { std::slice::from_raw_parts(self.backing.as_ptr().add(self.orig_ids_off) as *const u32, len) }
    }

    fn cluster_offsets(&self) -> &[u32] {
        let len = (self.k_clusters + 1) as usize;
        unsafe { std::slice::from_raw_parts(self.backing.as_ptr().add(self.cluster_offsets_off) as *const u32, len) }
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

    fn cluster_range(&self, cluster: u32) -> (usize, usize) {
        let offsets = self.cluster_offsets();
        let start = offsets[cluster as usize] as usize;
        let end = offsets[(cluster + 1) as usize] as usize;
        (start, end)
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

    fn push(&mut self, cand: Candidate) {
        if self.len < K {
            self.items[self.len] = cand;
            self.len += 1;
            self.items[..self.len].sort_unstable_by(compare_candidates);
        } else {
            let worst = &self.items[K - 1];
            if cand.dist < worst.dist || (cand.dist == worst.dist && cand.orig_id < worst.orig_id) {
                self.items[K - 1] = cand;
                self.items[..K].sort_unstable_by(compare_candidates);
            }
        }
    }

    fn max_dist(&self) -> u32 {
        if self.len < K { u32::MAX } else { self.items[K - 1].dist }
    }

    fn into_sorted_array(self) -> [Candidate; K] {
        self.items
    }
}

fn compare_candidates(a: &Candidate, b: &Candidate) -> Ordering {
    a.dist.cmp(&b.dist).then_with(|| a.orig_id.cmp(&b.orig_id))
}

fn l2_distance_i16_scalar(a: &[i16; 14], b: &[i16; 14]) -> u32 {
    let mut sum = 0u32;
    for i in 0..14 {
        let d = a[i] as i32 - b[i] as i32;
        sum += (d * d) as u32;
    }
    sum
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn l2_distance_i16_avx2(a: &[i16; 14], b: &[i16; 14]) -> u32 {
    use std::arch::x86_64::*;

    let a0 = _mm_loadu_si128(a.as_ptr() as *const __m128i);
    let b0 = _mm_loadu_si128(b.as_ptr() as *const __m128i);
    let d0 = _mm_sub_epi16(a0, b0);
    let s0 = _mm_madd_epi16(d0, d0);

    let mut sums = [0i32; 4];
    _mm_storeu_si128(sums.as_mut_ptr() as *mut __m128i, s0);
    let mut sum = (sums[0] + sums[1] + sums[2] + sums[3]) as u32;

    for i in 8..14 {
        let d = a[i] as i32 - b[i] as i32;
        sum += (d * d) as u32;
    }
    sum
}

#[cfg(not(target_arch = "x86_64"))]
fn l2_distance_i16_avx2(a: &[i16; 14], b: &[i16; 14]) -> u32 {
    l2_distance_i16_scalar(a, b)
}

pub fn l2_distance_i16(a: &[i16; 14], b: &[i16; 14]) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { return l2_distance_i16_avx2(a, b); }
        }
    }
    l2_distance_i16_scalar(a, b)
}

fn scan_cluster(index: &Index, query: &[i16; 14], cluster: u32, heap: &mut FixedHeap<5>) {
    let dim = index.dim as usize;
    let vectors = index.vectors();
    let labels = index.labels();
    let orig_ids = index.orig_ids();
    let (start, end) = index.cluster_range(cluster);

    for i in start..end {
        let vec_start = i * dim;
        let vec: &[i16] = &vectors[vec_start..vec_start + dim];
        let arr: &[i16; 14] = unsafe { &*(vec.as_ptr() as *const [i16; 14]) };
        let dist = l2_distance_i16(query, arr);
        let cand = Candidate {
            dist,
            orig_id: orig_ids[i],
            label: labels[i],
        };
        heap.push(cand);
    }
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
