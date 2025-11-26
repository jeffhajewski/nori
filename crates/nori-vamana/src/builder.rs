//! Vamana index builder.
//!
//! Builds a Vamana graph index from a set of vectors.

use crate::graph::{DiskGraphBuilder, NodeId};
use crate::pq::{PQConfig, ProductQuantizer};
use crate::Result;
use nori_vector::DistanceFunction;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Vamana build configuration.
#[derive(Debug, Clone)]
pub struct VamanaConfig {
    /// Maximum out-degree (R). Default: 64.
    pub max_degree: usize,

    /// Search list size during construction (L). Default: 100.
    pub search_list_size: usize,

    /// Pruning parameter (alpha). Default: 1.2.
    pub alpha: f32,

    /// Number of build passes. Default: 2.
    pub num_passes: usize,

    /// Product Quantization config.
    pub pq_config: PQConfig,
}

impl Default for VamanaConfig {
    fn default() -> Self {
        Self {
            max_degree: 64,
            search_list_size: 100,
            alpha: 1.2,
            num_passes: 2,
            pq_config: PQConfig::default(),
        }
    }
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

/// Vamana index builder.
pub struct VamanaBuilder {
    /// Vector dimensions
    dimensions: usize,

    /// Distance function
    distance: DistanceFunction,

    /// Configuration
    config: VamanaConfig,

    /// Vectors added so far
    vectors: Vec<Vec<f32>>,

    /// External IDs
    ids: Vec<String>,
}

impl VamanaBuilder {
    /// Create a new Vamana builder.
    pub fn new(dimensions: usize, distance: DistanceFunction, config: VamanaConfig) -> Self {
        Self {
            dimensions,
            distance,
            config,
            vectors: Vec::new(),
            ids: Vec::new(),
        }
    }

    /// Add a vector to be indexed.
    pub fn add(&mut self, id: &str, vector: &[f32]) -> Result<()> {
        if vector.len() != self.dimensions {
            return Err(crate::VamanaError::Build(format!(
                "Dimension mismatch: expected {}, got {}",
                self.dimensions,
                vector.len()
            )));
        }

        self.ids.push(id.to_string());
        self.vectors.push(vector.to_vec());
        Ok(())
    }

    /// Get number of vectors added.
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// Build the Vamana index.
    ///
    /// Writes multiple files:
    /// - `{path}.graph`: The Vamana graph
    /// - `{path}.pq`: Product Quantization codebook + codes
    /// - `{path}.vectors`: Full vectors (for re-ranking)
    /// - `{path}.ids`: Vector IDs
    pub fn build<P: AsRef<Path>>(self, path: P) -> Result<()> {
        let path = path.as_ref();

        if self.vectors.is_empty() {
            return Err(crate::VamanaError::Build("No vectors to build".to_string()));
        }

        // 1. Train Product Quantization
        let pq = ProductQuantizer::train(&self.vectors, self.dimensions, self.config.pq_config.clone())?;

        // 2. Encode all vectors
        let pq_codes: Vec<Vec<u8>> = self.vectors.iter().map(|v| pq.encode(v)).collect();

        // 3. Build Vamana graph
        let mut graph = DiskGraphBuilder::new(self.config.max_degree);

        // Add all nodes
        for _ in 0..self.vectors.len() {
            graph.add_node();
        }

        // Initialize with random edges
        self.initialize_random_graph(&mut graph);

        // Vamana pruning passes
        for pass in 0..self.config.num_passes {
            let alpha = if pass == 0 { 1.0 } else { self.config.alpha };
            self.prune_pass(&mut graph, alpha);
        }

        // 4. Write files
        // Graph
        graph.write(path.with_extension("graph"))?;

        // PQ codebook + codes
        self.write_pq(&pq, &pq_codes, path.with_extension("pq"))?;

        // Full vectors
        self.write_vectors(path.with_extension("vectors"))?;

        // IDs
        self.write_ids(path.with_extension("ids"))?;

        Ok(())
    }

    /// Initialize graph with random edges.
    fn initialize_random_graph(&self, graph: &mut DiskGraphBuilder) {
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();

        let n = self.vectors.len();
        let all_nodes: Vec<NodeId> = (0..n as NodeId).collect();

        for node in 0..n as NodeId {
            // Sample random neighbors
            let mut candidates: Vec<_> = all_nodes.iter().filter(|&&x| x != node).copied().collect();
            candidates.shuffle(&mut rng);

            let num_neighbors = self.config.max_degree.min(candidates.len());
            for &neighbor in &candidates[..num_neighbors] {
                graph.add_edge(node, neighbor);
            }
        }
    }

    /// Perform a pruning pass over all nodes.
    fn prune_pass(&self, graph: &mut DiskGraphBuilder, alpha: f32) {
        for node in 0..self.vectors.len() as NodeId {
            // Search for candidates
            let candidates = self.greedy_search(graph, node, self.config.search_list_size);

            // Robust prune
            let pruned = self.robust_prune(&candidates, node, alpha);

            // Update neighbors
            graph.set_neighbors(node, pruned.clone());

            // Add reverse edges
            for &neighbor in &pruned {
                let neighbor_neighbors = graph.neighbors(neighbor).to_vec();
                if neighbor_neighbors.len() < self.config.max_degree
                    && !neighbor_neighbors.contains(&node)
                {
                    graph.add_edge(neighbor, node);
                }
            }
        }
    }

    /// Greedy search in the graph.
    fn greedy_search(
        &self,
        graph: &DiskGraphBuilder,
        query_node: NodeId,
        list_size: usize,
    ) -> Vec<Candidate> {
        let query = &self.vectors[query_node as usize];

        // Start from a random entry point
        let entry = 0; // Could be randomized

        let mut candidates: BinaryHeap<Reverse<Candidate>> = BinaryHeap::new();
        let mut results: BinaryHeap<Candidate> = BinaryHeap::new();
        let mut visited: HashSet<NodeId> = HashSet::new();

        let entry_dist = self.compute_distance(query, entry);
        candidates.push(Reverse(Candidate {
            node_id: entry,
            distance: entry_dist,
        }));
        results.push(Candidate {
            node_id: entry,
            distance: entry_dist,
        });
        visited.insert(entry);

        while let Some(Reverse(current)) = candidates.pop() {
            if results.len() >= list_size {
                if let Some(worst) = results.peek() {
                    if current.distance > worst.distance {
                        break;
                    }
                }
            }

            for &neighbor in graph.neighbors(current.node_id) {
                if visited.contains(&neighbor) {
                    continue;
                }
                visited.insert(neighbor);

                let dist = self.compute_distance(query, neighbor);

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

    /// Robust pruning algorithm.
    fn robust_prune(
        &self,
        candidates: &[Candidate],
        node: NodeId,
        alpha: f32,
    ) -> Vec<NodeId> {
        let mut result = Vec::with_capacity(self.config.max_degree);
        let mut remaining: Vec<_> = candidates
            .iter()
            .filter(|c| c.node_id != node)
            .cloned()
            .collect();

        remaining.sort();

        while !remaining.is_empty() && result.len() < self.config.max_degree {
            let best = remaining.remove(0);
            result.push(best.node_id);

            // Filter remaining: keep only if not dominated
            remaining.retain(|c| {
                let dist_to_best = self.compute_distance(
                    &self.vectors[c.node_id as usize],
                    best.node_id,
                );
                // Keep if c is closer to node than alpha * dist_to_best
                c.distance < alpha * dist_to_best
            });
        }

        result
    }

    /// Compute distance between a query and a node.
    fn compute_distance(&self, query: &[f32], node: NodeId) -> f32 {
        self.distance.distance(query, &self.vectors[node as usize])
    }

    /// Write PQ data to file.
    fn write_pq<P: AsRef<Path>>(
        &self,
        pq: &ProductQuantizer,
        codes: &[Vec<u8>],
        path: P,
    ) -> Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        // Write codebook
        let codebook_bytes = pq.to_bytes();
        writer.write_all(&(codebook_bytes.len() as u64).to_le_bytes())?;
        writer.write_all(&codebook_bytes)?;

        // Write codes
        writer.write_all(&(codes.len() as u64).to_le_bytes())?;
        writer.write_all(&(pq.num_subspaces() as u32).to_le_bytes())?;
        for code in codes {
            writer.write_all(code)?;
        }

        writer.flush()?;
        Ok(())
    }

    /// Write full vectors to file.
    fn write_vectors<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        writer.write_all(&(self.vectors.len() as u64).to_le_bytes())?;
        writer.write_all(&(self.dimensions as u32).to_le_bytes())?;

        for vector in &self.vectors {
            for &val in vector {
                writer.write_all(&val.to_le_bytes())?;
            }
        }

        writer.flush()?;
        Ok(())
    }

    /// Write IDs to file.
    fn write_ids<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        writer.write_all(&(self.ids.len() as u64).to_le_bytes())?;

        for id in &self.ids {
            let id_bytes = id.as_bytes();
            writer.write_all(&(id_bytes.len() as u32).to_le_bytes())?;
            writer.write_all(id_bytes)?;
        }

        writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn generate_test_vectors(n: usize, dims: usize) -> Vec<Vec<f32>> {
        (0..n)
            .map(|i| (0..dims).map(|j| ((i * j) % 100) as f32 / 100.0).collect())
            .collect()
    }

    #[test]
    fn test_builder_basic() {
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

        assert_eq!(builder.len(), 100);

        let dir = tempdir().unwrap();
        let path = dir.path().join("index");
        builder.build(&path).unwrap();

        // Verify files exist
        assert!(path.with_extension("graph").exists());
        assert!(path.with_extension("pq").exists());
        assert!(path.with_extension("vectors").exists());
        assert!(path.with_extension("ids").exists());
    }
}
