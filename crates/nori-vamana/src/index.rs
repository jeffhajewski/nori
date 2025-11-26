//! Vamana index for search operations.
//!
//! Provides disk-resident vector search using the Vamana graph
//! with two-phase search: PQ beam search + exact re-ranking.

use crate::graph::{DiskGraph, NodeId};
use crate::pq::ProductQuantizer;
use crate::Result;
use nori_vector::{DistanceFunction, VectorMatch};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

/// Vamana index for disk-resident vector search.
pub struct VamanaIndex {
    /// Disk-resident graph
    graph: DiskGraph,

    /// Product Quantizer
    pq: ProductQuantizer,

    /// PQ codes for all vectors
    pq_codes: Vec<Vec<u8>>,

    /// Full vectors (memory-mapped or loaded)
    vectors: Vec<Vec<f32>>,

    /// Vector IDs
    ids: Vec<String>,

    /// Distance function
    distance: DistanceFunction,

    /// Dimensions
    dimensions: usize,

    /// Re-rank multiplier (how many candidates to fetch for re-ranking)
    rerank_multiplier: usize,
}

/// Candidate during search.
#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    node_id: NodeId,
    distance: f32,
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.distance.total_cmp(&other.distance)
    }
}

impl VamanaIndex {
    /// Open a Vamana index from disk.
    ///
    /// Expects files:
    /// - `{path}.graph`: The Vamana graph
    /// - `{path}.pq`: Product Quantization codebook + codes
    /// - `{path}.vectors`: Full vectors
    /// - `{path}.ids`: Vector IDs
    pub fn open<P: AsRef<Path>>(path: P, distance: DistanceFunction) -> Result<Self> {
        let path = path.as_ref();

        // Load graph
        let graph = DiskGraph::open(path.with_extension("graph"))?;

        // Load PQ
        let (pq, pq_codes) = Self::load_pq(path.with_extension("pq"))?;

        // Load vectors
        let (vectors, dimensions) = Self::load_vectors(path.with_extension("vectors"))?;

        // Load IDs
        let ids = Self::load_ids(path.with_extension("ids"))?;

        Ok(Self {
            graph,
            pq,
            pq_codes,
            vectors,
            ids,
            distance,
            dimensions,
            rerank_multiplier: 10,
        })
    }

    /// Set the re-rank multiplier.
    ///
    /// When searching for k results, we first find k * rerank_multiplier
    /// candidates using PQ distance, then re-rank with exact distance.
    pub fn set_rerank_multiplier(&mut self, multiplier: usize) {
        self.rerank_multiplier = multiplier;
    }

    /// Search for the k nearest neighbors.
    ///
    /// Uses two-phase search:
    /// 1. Beam search with PQ distances (fast, approximate)
    /// 2. Re-rank top candidates with exact distances
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<VectorMatch>> {
        if k == 0 || self.vectors.is_empty() {
            return Ok(vec![]);
        }

        // Phase 1: PQ beam search
        let candidate_count = k * self.rerank_multiplier;
        let candidates = self.beam_search_pq(query, candidate_count);

        // Phase 2: Re-rank with exact distances
        let mut results: Vec<_> = candidates
            .into_iter()
            .map(|c| {
                let exact_dist = self.distance.distance(query, &self.vectors[c.node_id as usize]);
                VectorMatch::new(self.ids[c.node_id as usize].clone(), exact_dist)
            })
            .collect();

        results.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        results.truncate(k);

        Ok(results)
    }

    /// Beam search using PQ distances.
    fn beam_search_pq(&self, query: &[f32], list_size: usize) -> Vec<Candidate> {
        // Precompute distance table for fast PQ distance
        let distance_table = self.pq.precompute_distance_table(query);

        let mut candidates: BinaryHeap<Reverse<Candidate>> = BinaryHeap::new();
        let mut results: BinaryHeap<Candidate> = BinaryHeap::new();
        let mut visited: HashSet<NodeId> = HashSet::new();

        // Start from node 0 (could use a medoid for better performance)
        let entry = 0;
        let entry_dist = self.pq.distance_from_table(&distance_table, &self.pq_codes[entry]);

        candidates.push(Reverse(Candidate {
            node_id: entry as NodeId,
            distance: entry_dist,
        }));
        results.push(Candidate {
            node_id: entry as NodeId,
            distance: entry_dist,
        });
        visited.insert(entry as NodeId);

        while let Some(Reverse(current)) = candidates.pop() {
            if results.len() >= list_size {
                if let Some(worst) = results.peek() {
                    if current.distance > worst.distance {
                        break;
                    }
                }
            }

            for neighbor in self.graph.neighbors(current.node_id) {
                if visited.contains(&neighbor) {
                    continue;
                }
                visited.insert(neighbor);

                let dist = self.pq.distance_from_table(&distance_table, &self.pq_codes[neighbor as usize]);

                let should_add =
                    results.len() < list_size || results.peek().map(|w| dist < w.distance).unwrap_or(true);

                if should_add {
                    candidates.push(Reverse(Candidate {
                        node_id: neighbor,
                        distance: dist,
                    }));
                    results.push(Candidate {
                        node_id: neighbor,
                        distance: dist,
                    });

                    while results.len() > list_size {
                        results.pop();
                    }
                }
            }
        }

        let mut result_vec: Vec<_> = results.into_iter().collect();
        result_vec.sort();
        result_vec
    }

    /// Get a vector by ID.
    pub fn get(&self, id: &str) -> Option<&[f32]> {
        self.ids
            .iter()
            .position(|x| x == id)
            .map(|idx| self.vectors[idx].as_slice())
    }

    /// Get the number of vectors.
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// Get dimensions.
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// Load PQ data from file.
    fn load_pq<P: AsRef<Path>>(path: P) -> Result<(ProductQuantizer, Vec<Vec<u8>>)> {
        let mut file = BufReader::new(File::open(path)?);

        // Read codebook
        let mut len_bytes = [0u8; 8];
        file.read_exact(&mut len_bytes)?;
        let codebook_len = u64::from_le_bytes(len_bytes) as usize;

        let mut codebook_bytes = vec![0u8; codebook_len];
        file.read_exact(&mut codebook_bytes)?;

        let pq = ProductQuantizer::from_bytes(&codebook_bytes)?;

        // Read codes
        let mut num_codes_bytes = [0u8; 8];
        file.read_exact(&mut num_codes_bytes)?;
        let num_codes = u64::from_le_bytes(num_codes_bytes) as usize;

        let mut code_len_bytes = [0u8; 4];
        file.read_exact(&mut code_len_bytes)?;
        let code_len = u32::from_le_bytes(code_len_bytes) as usize;

        let mut codes = Vec::with_capacity(num_codes);
        for _ in 0..num_codes {
            let mut code = vec![0u8; code_len];
            file.read_exact(&mut code)?;
            codes.push(code);
        }

        Ok((pq, codes))
    }

    /// Load vectors from file.
    fn load_vectors<P: AsRef<Path>>(path: P) -> Result<(Vec<Vec<f32>>, usize)> {
        let mut file = BufReader::new(File::open(path)?);

        let mut header = [0u8; 12];
        file.read_exact(&mut header)?;

        let num_vectors = u64::from_le_bytes([
            header[0], header[1], header[2], header[3],
            header[4], header[5], header[6], header[7],
        ]) as usize;

        let dimensions = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;

        let mut vectors = Vec::with_capacity(num_vectors);
        for _ in 0..num_vectors {
            let mut vector = Vec::with_capacity(dimensions);
            for _ in 0..dimensions {
                let mut val_bytes = [0u8; 4];
                file.read_exact(&mut val_bytes)?;
                vector.push(f32::from_le_bytes(val_bytes));
            }
            vectors.push(vector);
        }

        Ok((vectors, dimensions))
    }

    /// Load IDs from file.
    fn load_ids<P: AsRef<Path>>(path: P) -> Result<Vec<String>> {
        let mut file = BufReader::new(File::open(path)?);

        let mut num_ids_bytes = [0u8; 8];
        file.read_exact(&mut num_ids_bytes)?;
        let num_ids = u64::from_le_bytes(num_ids_bytes) as usize;

        let mut ids = Vec::with_capacity(num_ids);
        for _ in 0..num_ids {
            let mut len_bytes = [0u8; 4];
            file.read_exact(&mut len_bytes)?;
            let len = u32::from_le_bytes(len_bytes) as usize;

            let mut id_bytes = vec![0u8; len];
            file.read_exact(&mut id_bytes)?;
            ids.push(String::from_utf8_lossy(&id_bytes).to_string());
        }

        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{VamanaBuilder, VamanaConfig};
    use crate::pq::PQConfig;
    use tempfile::tempdir;

    fn generate_test_vectors(n: usize, dims: usize) -> Vec<Vec<f32>> {
        (0..n)
            .map(|i| (0..dims).map(|j| ((i * j) % 100) as f32 / 100.0).collect())
            .collect()
    }

    #[test]
    fn test_index_build_and_search() {
        let config = VamanaConfig {
            max_degree: 8,
            search_list_size: 20,
            num_passes: 1,
            pq_config: PQConfig {
                num_subspaces: 4,
                bits_per_code: 8,
                kmeans_iterations: 5,
                training_sample_size: 100,
            },
            ..Default::default()
        };

        let mut builder = VamanaBuilder::new(32, DistanceFunction::Euclidean, config);

        let vectors = generate_test_vectors(100, 32);
        for (i, vec) in vectors.iter().enumerate() {
            builder.add(&format!("vec{}", i), vec).unwrap();
        }

        let dir = tempdir().unwrap();
        let path = dir.path().join("index");
        builder.build(&path).unwrap();

        // Open and search
        let index = VamanaIndex::open(&path, DistanceFunction::Euclidean).unwrap();
        assert_eq!(index.len(), 100);
        assert_eq!(index.dimensions(), 32);

        // Search for vector similar to vec0
        let results = index.search(&vectors[0], 5).unwrap();
        assert_eq!(results.len(), 5);

        // vec0 should be in results (exact match or very close)
        assert!(results.iter().any(|r| r.id == "vec0" || r.distance < 0.1));

        // Results should be sorted by distance
        for i in 1..results.len() {
            assert!(results[i - 1].distance <= results[i].distance);
        }
    }

    #[test]
    fn test_index_get() {
        let config = VamanaConfig {
            max_degree: 8,
            search_list_size: 20,
            num_passes: 1,
            pq_config: PQConfig {
                num_subspaces: 4,
                bits_per_code: 8,
                kmeans_iterations: 5,
                training_sample_size: 50,
            },
            ..Default::default()
        };

        let mut builder = VamanaBuilder::new(32, DistanceFunction::Euclidean, config);

        let vectors = generate_test_vectors(50, 32);
        for (i, vec) in vectors.iter().enumerate() {
            builder.add(&format!("vec{}", i), vec).unwrap();
        }

        let dir = tempdir().unwrap();
        let path = dir.path().join("index");
        builder.build(&path).unwrap();

        let index = VamanaIndex::open(&path, DistanceFunction::Euclidean).unwrap();

        // Get vector by ID
        let vec0 = index.get("vec0").unwrap();
        assert_eq!(vec0.len(), 32);

        // Non-existent ID
        assert!(index.get("nonexistent").is_none());
    }
}
