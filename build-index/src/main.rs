use flate2::read::GzDecoder;
use serde::Deserialize;
use std::env;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Deserialize)]
struct Record {
    vector: Vec<f32>,
    label: String,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: build-index <resources_dir> <output_path>");
        std::process::exit(1);
    }

    let resources_dir = &args[1];
    let output_path = &args[2];

    const K: usize = 1024;
    const DIM: usize = 14;
    const SCALE: u32 = 10000;
    const MAX_ITER: usize = 20;

    // --- 1. Load data ---
    eprintln!("Reading references.json.gz...");
    let gz_path = Path::new(resources_dir).join("references.json.gz");
    let gz_file = fs::File::open(&gz_path).expect("Cannot open references.json.gz");
    let decoder = GzDecoder::new(gz_file);
    let records: Vec<Record> = serde_json::from_reader(decoder).expect("Failed to parse JSON");

    let n = records.len();
    let mut vectors: Vec<[f32; 14]> = Vec::with_capacity(n);
    let mut labels: Vec<u8> = Vec::with_capacity(n);

    for rec in &records {
        let mut v = [0.0f32; 14];
        let len = rec.vector.len().min(DIM);
        for i in 0..len {
            v[i] = rec.vector[i];
        }
        vectors.push(v);
        labels.push(if rec.label == "fraud" { 1 } else { 0 });
    }
    drop(records);

    let n = vectors.len();
    eprintln!("Loaded {} vectors", n);

    // --- 2. K-Means ---
    eprintln!("Running K-Means (K={}, {} iterations)...", K, MAX_ITER);

    let mut centroids: Vec<[f32; 14]> = Vec::with_capacity(K);
    for i in 0..K.min(n) {
        centroids.push(vectors[i]);
    }
    while centroids.len() < K {
        centroids.push([0.0f32; 14]);
    }

    let mut cluster_of: Vec<u32> = vec![0; n];

    for iteration in 0..MAX_ITER {
        // ASSIGN
        let mut inertia = 0.0f64;

        for i in 0..n {
            let vec = &vectors[i];
            let mut best_dist = f64::MAX;
            let mut best_k = 0u32;

            for k in 0..K {
                let cent = &centroids[k];
                let mut sum = 0.0f64;
                for d in 0..DIM {
                    let diff = vec[d] as f64 - cent[d] as f64;
                    sum += diff * diff;
                }
                if sum < best_dist {
                    best_dist = sum;
                    best_k = k as u32;
                }
            }

            cluster_of[i] = best_k;
            inertia += best_dist;
        }

        // UPDATE
        let mut new_centroids: Vec<[f64; 14]> = vec![[0.0f64; 14]; K];
        let mut counts: Vec<u64> = vec![0; K];

        for i in 0..n {
            let vec = &vectors[i];
            let k = cluster_of[i] as usize;
            for d in 0..DIM {
                new_centroids[k][d] += vec[d] as f64;
            }
            counts[k] += 1;
        }

        // Handle empty clusters
        for k in 0..K {
            if counts[k] == 0 {
                let mut max_dist = -1.0f64;
                let mut farthest = 0usize;

                for i in 0..n {
                    let ck = cluster_of[i] as usize;
                    let vec = &vectors[i];
                    let cent = &centroids[ck];
                    let mut dist = 0.0f64;
                    for d in 0..DIM {
                        let diff = vec[d] as f64 - cent[d] as f64;
                        dist += diff * diff;
                    }
                    if dist > max_dist {
                        max_dist = dist;
                        farthest = i;
                    }
                }

                let old_k = cluster_of[farthest] as usize;
                cluster_of[farthest] = k as u32;

                for d in 0..DIM {
                    let val = vectors[farthest][d] as f64;
                    new_centroids[old_k][d] -= val;
                    new_centroids[k][d] += val;
                }
                counts[old_k] -= 1;
                counts[k] = 1;
            }
        }

        // Compute new centroids
        for k in 0..K {
            if counts[k] > 0 {
                let inv = 1.0 / counts[k] as f64;
                let c = &mut centroids[k];
                for d in 0..DIM {
                    c[d] = (new_centroids[k][d] * inv) as f32;
                }
            }
        }

        eprintln!("iteration {}/{} inertia={:.6}", iteration + 1, MAX_ITER, inertia);
    }

    // --- 3. Quantize to i16 ---
    eprintln!("Quantizing vectors...");
    let scale_f = SCALE as f32;
    let mut vectors_i16: Vec<[i16; 14]> = Vec::with_capacity(n);
    for i in 0..n {
        let vec = &vectors[i];
        let mut q = [0i16; 14];
        for d in 0..DIM {
            let scaled = vec[d] * scale_f;
            let rounded = scaled.round();
            q[d] = if rounded > 32767.0 {
                32767i16
            } else if rounded < -32768.0 {
                -32768i16
            } else {
                rounded as i16
            };
        }
        vectors_i16.push(q);
    }

    drop(vectors);

    // --- 4. Sort by cluster, preserve orig_id order ---
    eprintln!("Sorting by cluster...");

    let mut cluster_lists: Vec<Vec<(u32, [i16; 14], u8)>> = (0..K).map(|_| Vec::new()).collect();
    for i in 0..n {
        let k = cluster_of[i] as usize;
        cluster_lists[k].push((i as u32, vectors_i16[i], labels[i]));
    }

    for list in &mut cluster_lists {
        list.sort_by_key(|item| item.0);
    }

    let mut sorted_vec: Vec<[i16; 14]> = Vec::with_capacity(n);
    let mut sorted_lbl: Vec<u8> = Vec::with_capacity(n);
    let mut sorted_oid: Vec<u32> = Vec::with_capacity(n);
    let mut sorted_clu: Vec<u32> = Vec::with_capacity(n);
    let mut cluster_offsets: Vec<u32> = Vec::with_capacity(K + 1);

    let mut offset: u32 = 0;
    cluster_offsets.push(0);
    for k in 0..K {
        for &(orig_id, ref vec, label) in &cluster_lists[k] {
            sorted_vec.push(*vec);
            sorted_lbl.push(label);
            sorted_oid.push(orig_id);
            sorted_clu.push(k as u32);
        }
        offset += cluster_lists[k].len() as u32;
        cluster_offsets.push(offset);
    }

    // --- 5. Compute bbox per cluster ---
    eprintln!("Computing bounding boxes...");
    let mut bbox_min: Vec<[i16; 14]> = vec![[i16::MAX; 14]; K];
    let mut bbox_max: Vec<[i16; 14]> = vec![[i16::MIN; 14]; K];

    for k in 0..K {
        for &(_, ref vec, _) in &cluster_lists[k] {
            for d in 0..DIM {
                if vec[d] < bbox_min[k][d] {
                    bbox_min[k][d] = vec[d];
                }
                if vec[d] > bbox_max[k][d] {
                    bbox_max[k][d] = vec[d];
                }
            }
        }
    }

    drop(cluster_lists);
    drop(labels);
    drop(vectors_i16);

    // --- 6. Write index.bin v2 (SoA layout) ---
    eprintln!("Writing index.bin...");
    let file = fs::File::create(output_path).expect("Cannot create output file");
    let mut w = BufWriter::new(file);

    // Header (version 2)
    w.write_all(b"RVIF").unwrap();
    w.write_all(&2u32.to_le_bytes()).unwrap();
    w.write_all(&(n as u32).to_le_bytes()).unwrap();
    w.write_all(&(DIM as u32).to_le_bytes()).unwrap();
    w.write_all(&(K as u32).to_le_bytes()).unwrap();
    w.write_all(&SCALE.to_le_bytes()).unwrap();
    w.write_all(&[0u8; 40]).unwrap();

    // Centroids f32 (row-major [K][DIM])
    for k in 0..K {
        for d in 0..DIM {
            w.write_all(&centroids[k][d].to_le_bytes()).unwrap();
        }
    }

    // Vectors i16 SoA per cluster, padded to 8
    let mut actual_counts: Vec<u32> = Vec::with_capacity(K);
    let mut soa_byte_offsets: Vec<u32> = Vec::with_capacity(K + 1);
    soa_byte_offsets.push(0);
    let mut total_padded: usize = 0;

    for k in 0..K {
        let start = cluster_offsets[k as usize] as usize;
        let end = cluster_offsets[(k + 1) as usize] as usize;
        let actual = end - start;
        let padded = ((actual + 7) / 8) * 8;
        actual_counts.push(actual as u32);

        for d in 0..DIM {
            for i in 0..padded {
                if i < actual {
                    w.write_all(&sorted_vec[start + i][d].to_le_bytes()).unwrap();
                } else {
                    w.write_all(&i16::MAX.to_le_bytes()).unwrap();
                }
            }
        }

        let chunk = padded * DIM * 2;
        total_padded += chunk;
        soa_byte_offsets.push(total_padded as u32);
    }

    // Labels u8 (0=legit, 1=fraud)
    w.write_all(&sorted_lbl).unwrap();

    // orig_ids u32
    for &id in &sorted_oid {
        w.write_all(&id.to_le_bytes()).unwrap();
    }

    // ClusterCounts u32 [K]
    for &count in &actual_counts {
        w.write_all(&count.to_le_bytes()).unwrap();
    }

    // cluster_of u32
    for &c in &sorted_clu {
        w.write_all(&c.to_le_bytes()).unwrap();
    }

    // cluster_byte_offsets u32 prefix-sum [K+1] (SoA byte offsets)
    for &off in &soa_byte_offsets {
        w.write_all(&off.to_le_bytes()).unwrap();
    }

    // bbox_min i16 [K][DIM]
    for k in 0..K {
        for d in 0..DIM {
            w.write_all(&bbox_min[k][d].to_le_bytes()).unwrap();
        }
    }

    // bbox_max i16 [K][DIM]
    for k in 0..K {
        for d in 0..DIM {
            w.write_all(&bbox_max[k][d].to_le_bytes()).unwrap();
        }
    }

    w.flush().unwrap();

    let c_size = K * DIM * 4;
    eprintln!("Index written: N={} K={} dim={} scale={} (v2 SoA)", n, K, DIM, SCALE);
    eprintln!("Centroid block: {}*{}*{} = {} bytes", K, DIM, 4, c_size);
    eprintln!("Vectors SoA block: {} total_padded={}", total_padded, total_padded / 2 / DIM);
}
