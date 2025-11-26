//! HNSW index implementation.
//!
//! Implements the Hierarchical Navigable Small World algorithm for
//! approximate nearest neighbor search.

use crate::graph::{Neighbors, Node, NodeId};
use crate::layer::{random_layer, Layers};
use crate::Result;
use nori_vector::{DistanceFunction, VectorError, VectorIndex, VectorMatch};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// HNSW configuration parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HnswConfig {
    /// Max connections per node (M parameter).
    /// Default: 16
    pub m: usize,

    /// Max connections in layer 0 (M_max0 = 2*M typically).
    /// Default: 32
    pub m_max0: usize,

    /// Beam width during construction (ef_construction).
    /// Higher = better quality, slower build.
    /// Default: 200
    pub ef_construction: usize,

    /// Beam width during search (ef_search).
    /// Higher = better recall, slower search.
    /// Default: 100
    pub ef_search: usize,

    /// Maximum number of layers.
    /// Default: 16 (supports ~10^7 vectors)
    pub max_layers: usize,
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self {
            m: 16,
            m_max0: 32,
            ef_construction: 200,
            ef_search: 100,
            max_layers: 16,
        }
    }
}

/// Candidate during search (node_id, distance).
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

/// HNSW index.
///
/// Thread-safe HNSW implementation supporting concurrent reads and writes.
pub struct HnswIndex {
    /// Configuration
    config: HnswConfig,

    /// Vector dimensions
    dimensions: usize,

    /// Distance function
    distance: DistanceFunction,

    /// Multi-layer graph structure
    layers: Layers,

    /// Node storage: internal_id -> Node
    nodes: RwLock<Vec<Node>>,

    /// External ID -> internal ID mapping
    id_map: RwLock<HashMap<String, NodeId>>,

    /// Entry point (highest layer node)
    entry_point: RwLock<Option<NodeId>>,

    /// Current max layer in use
    max_layer: RwLock<usize>,
}

impl HnswIndex {
    /// Create a new HNSW index.
    ///
    /// # Arguments
    ///
    /// * `dimensions` - Vector dimensions
    /// * `distance` - Distance function to use
    /// * `config` - HNSW configuration parameters
    pub fn new(dimensions: usize, distance: DistanceFunction, config: HnswConfig) -> Self {
        let layers = Layers::new(config.max_layers, config.m, config.m_max0);

        Self {
            config,
            dimensions,
            distance,
            layers,
            nodes: RwLock::new(Vec::new()),
            id_map: RwLock::new(HashMap::new()),
            entry_point: RwLock::new(None),
            max_layer: RwLock::new(0),
        }
    }

    /// Get configuration.
    pub fn config(&self) -> &HnswConfig {
        &self.config
    }

    /// Compute distance between a query and a node.
    fn distance_to_node(&self, query: &[f32], node_id: NodeId) -> f32 {
        let nodes = self.nodes.read();
        if let Some(node) = nodes.get(node_id as usize) {
            self.distance.distance(query, &node.vector)
        } else {
            f32::MAX
        }
    }

    /// Search a single layer for nearest neighbors.
    ///
    /// Returns candidates sorted by distance (closest first).
    fn search_layer(
        &self,
        query: &[f32],
        entry_points: Vec<NodeId>,
        ef: usize,
        layer: usize,
    ) -> Vec<Candidate> {
        // Min-heap for candidates (closest first)
        let mut candidates: BinaryHeap<Reverse<Candidate>> = BinaryHeap::new();
        // Max-heap for results (furthest first, for pruning)
        let mut results: BinaryHeap<Candidate> = BinaryHeap::new();
        // Visited set
        let mut visited: HashSet<NodeId> = HashSet::new();

        // Initialize with entry points
        for ep in entry_points {
            let dist = self.distance_to_node(query, ep);
            candidates.push(Reverse(Candidate {
                node_id: ep,
                distance: dist,
            }));
            results.push(Candidate {
                node_id: ep,
                distance: dist,
            });
            visited.insert(ep);
        }

        while let Some(Reverse(current)) = candidates.pop() {
            // Stop if current is worse than worst in results (and results is full)
            if results.len() >= ef {
                if let Some(worst) = results.peek() {
                    if current.distance > worst.distance {
                        break;
                    }
                }
            }

            // Explore neighbors
            if let Some(layer_graph) = self.layers.get(layer) {
                if let Some(neighbors) = layer_graph.neighbors(current.node_id) {
                    for &neighbor_id in neighbors.iter() {
                        if visited.contains(&neighbor_id) {
                            continue;
                        }
                        visited.insert(neighbor_id);

                        let dist = self.distance_to_node(query, neighbor_id);

                        // Add to candidates if better than worst result
                        let should_add = results.len() < ef
                            || results
                                .peek()
                                .map(|w| dist < w.distance)
                                .unwrap_or(true);

                        if should_add {
                            candidates.push(Reverse(Candidate {
                                node_id: neighbor_id,
                                distance: dist,
                            }));
                            results.push(Candidate {
                                node_id: neighbor_id,
                                distance: dist,
                            });

                            // Prune results if over ef
                            while results.len() > ef {
                                results.pop();
                            }
                        }
                    }
                }
            }
        }

        // Convert results to sorted vec (closest first)
        let mut result_vec: Vec<_> = results.into_iter().collect();
        result_vec.sort();
        result_vec
    }

    /// Select neighbors using simple heuristic.
    ///
    /// Takes candidates and returns up to M neighbors.
    fn select_neighbors(&self, candidates: Vec<Candidate>, m: usize) -> Vec<NodeId> {
        // Simple approach: take M closest
        candidates
            .into_iter()
            .take(m)
            .map(|c| c.node_id)
            .collect()
    }

    /// Connect a new node to its neighbors at a specific layer.
    fn connect_node(&self, node_id: NodeId, neighbors: Vec<NodeId>, layer: usize) {
        if let Some(layer_graph) = self.layers.get(layer) {
            let max_degree = layer_graph.max_degree();

            // Add edges to neighbors
            for &neighbor_id in &neighbors {
                layer_graph.add_edge(node_id, neighbor_id);
            }

            // Shrink neighbor's connections if over max_degree
            for &neighbor_id in &neighbors {
                if let Some(neighbor_neighbors) = layer_graph.neighbors(neighbor_id) {
                    if neighbor_neighbors.len() > max_degree {
                        // Get all neighbors with distances
                        let nodes = self.nodes.read();
                        if let Some(neighbor_node) = nodes.get(neighbor_id as usize) {
                            let mut scored: Vec<_> = neighbor_neighbors
                                .iter()
                                .filter_map(|&nn_id| {
                                    nodes.get(nn_id as usize).map(|nn| {
                                        let dist = self
                                            .distance
                                            .distance(&neighbor_node.vector, &nn.vector);
                                        Candidate {
                                            node_id: nn_id,
                                            distance: dist,
                                        }
                                    })
                                })
                                .collect();

                            scored.sort();
                            let new_neighbors: Vec<_> =
                                scored.into_iter().take(max_degree).map(|c| c.node_id).collect();

                            let mut new_nn = Neighbors::with_capacity(max_degree);
                            for id in new_neighbors {
                                new_nn.add(id);
                            }
                            layer_graph.set_neighbors(neighbor_id, new_nn);
                        }
                    }
                }
            }
        }
    }

    /// Internal insert implementation.
    fn insert_internal(&self, external_id: &str, vector: &[f32]) -> Result<()> {
        // Allocate new node ID
        let node_id = {
            let mut nodes = self.nodes.write();
            let id = nodes.len() as NodeId;
            let node_layer = random_layer(self.config.m, self.config.max_layers);
            nodes.push(Node {
                external_id: external_id.to_string(),
                vector: vector.to_vec(),
                max_layer: node_layer,
            });
            id
        };

        // Update ID map
        {
            let mut id_map = self.id_map.write();
            id_map.insert(external_id.to_string(), node_id);
        }

        // Get node's max layer
        let node_layer = {
            let nodes = self.nodes.read();
            nodes[node_id as usize].max_layer
        };

        // Get current entry point and max layer
        let (entry_point, current_max_layer) = {
            let ep = *self.entry_point.read();
            let ml = *self.max_layer.read();
            (ep, ml)
        };

        // If first node, set as entry point
        if entry_point.is_none() {
            *self.entry_point.write() = Some(node_id);
            *self.max_layer.write() = node_layer;

            // Add node to all layers up to its max_layer
            for layer in 0..=node_layer {
                if let Some(layer_graph) = self.layers.get(layer) {
                    layer_graph.set_neighbors(node_id, Neighbors::new());
                }
            }
            return Ok(());
        }

        let mut ep = vec![entry_point.unwrap()];

        // Search from top layer down to node_layer + 1
        for layer in (node_layer + 1..=current_max_layer).rev() {
            let candidates = self.search_layer(vector, ep.clone(), 1, layer);
            if !candidates.is_empty() {
                ep = vec![candidates[0].node_id];
            }
        }

        // Insert into each layer from node_layer down to 0
        for layer in (0..=node_layer.min(current_max_layer)).rev() {
            let candidates = self.search_layer(vector, ep.clone(), self.config.ef_construction, layer);

            let m = if layer == 0 {
                self.config.m_max0
            } else {
                self.config.m
            };
            let neighbors = self.select_neighbors(candidates.clone(), m);

            // Add node to layer and connect
            if let Some(layer_graph) = self.layers.get(layer) {
                layer_graph.set_neighbors(node_id, Neighbors::new());
            }
            self.connect_node(node_id, neighbors, layer);

            // Use closest found as entry points for next layer
            ep = candidates.into_iter().take(1).map(|c| c.node_id).collect();
            if ep.is_empty() {
                ep = vec![entry_point.unwrap()];
            }
        }

        // Update entry point if new node has higher layer
        if node_layer > current_max_layer {
            *self.entry_point.write() = Some(node_id);
            *self.max_layer.write() = node_layer;
        }

        Ok(())
    }

    /// Validate vector dimensions and values.
    fn validate_vector(&self, vector: &[f32]) -> std::result::Result<(), VectorError> {
        if vector.len() != self.dimensions {
            return Err(VectorError::DimensionMismatch {
                expected: self.dimensions,
                actual: vector.len(),
            });
        }

        for (i, &v) in vector.iter().enumerate() {
            if v.is_nan() {
                return Err(VectorError::InvalidVector(format!("NaN at index {}", i)));
            }
            if v.is_infinite() {
                return Err(VectorError::InvalidVector(format!("Inf at index {}", i)));
            }
        }

        Ok(())
    }
}

impl VectorIndex for HnswIndex {
    fn insert(&self, id: &str, vector: &[f32]) -> nori_vector::Result<()> {
        self.validate_vector(vector)?;

        // Remove existing if present
        if self.contains(id) {
            self.delete(id)?;
        }

        self.insert_internal(id, vector)
            .map_err(|e| VectorError::IndexError(e.to_string()))
    }

    fn delete(&self, id: &str) -> nori_vector::Result<bool> {
        let node_id = {
            let mut id_map = self.id_map.write();
            match id_map.remove(id) {
                Some(id) => id,
                None => return Ok(false),
            }
        };

        // Remove from all layers
        self.layers.remove_node(node_id);

        // Note: We don't actually remove from nodes vec to keep IDs stable
        // In a production implementation, we might want to compact periodically

        // Update entry point if needed
        {
            let ep = *self.entry_point.read();
            if ep == Some(node_id) {
                // Find a new entry point
                let nodes = self.nodes.read();
                let id_map = self.id_map.read();
                let new_ep = id_map.values().next().copied();
                *self.entry_point.write() = new_ep;

                if let Some(new_ep_id) = new_ep {
                    if let Some(node) = nodes.get(new_ep_id as usize) {
                        *self.max_layer.write() = node.max_layer;
                    }
                }
            }
        }

        Ok(true)
    }

    fn search(&self, query: &[f32], k: usize) -> nori_vector::Result<Vec<VectorMatch>> {
        self.validate_vector(query)?;

        if k == 0 {
            return Ok(vec![]);
        }

        let entry_point = *self.entry_point.read();
        let entry_point = match entry_point {
            Some(ep) => ep,
            None => return Ok(vec![]), // Empty index
        };

        let current_max_layer = *self.max_layer.read();
        let mut ep = vec![entry_point];

        // Search from top layer down to layer 1
        for layer in (1..=current_max_layer).rev() {
            let candidates = self.search_layer(query, ep.clone(), 1, layer);
            if !candidates.is_empty() {
                ep = vec![candidates[0].node_id];
            }
        }

        // Search layer 0 with ef_search
        let candidates = self.search_layer(query, ep, self.config.ef_search, 0);

        // Convert to VectorMatch, take top k
        let nodes = self.nodes.read();
        let results: Vec<_> = candidates
            .into_iter()
            .take(k)
            .filter_map(|c| {
                nodes.get(c.node_id as usize).map(|node| {
                    VectorMatch::new(node.external_id.clone(), c.distance)
                })
            })
            .collect();

        Ok(results)
    }

    fn get(&self, id: &str) -> nori_vector::Result<Option<Vec<f32>>> {
        let id_map = self.id_map.read();
        let node_id = match id_map.get(id) {
            Some(&id) => id,
            None => return Ok(None),
        };

        let nodes = self.nodes.read();
        Ok(nodes.get(node_id as usize).map(|n| n.vector.clone()))
    }

    fn contains(&self, id: &str) -> bool {
        let id_map = self.id_map.read();
        id_map.contains_key(id)
    }

    fn len(&self) -> usize {
        let id_map = self.id_map.read();
        id_map.len()
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn clear(&self) {
        // Clear all state
        *self.nodes.write() = Vec::new();
        *self.id_map.write() = HashMap::new();
        *self.entry_point.write() = None;
        *self.max_layer.write() = 0;

        // Recreate layers
        // Note: In a real implementation, we'd clear existing layers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_index() -> HnswIndex {
        let config = HnswConfig {
            m: 8,
            m_max0: 16,
            ef_construction: 50,
            ef_search: 20,
            max_layers: 8,
        };
        HnswIndex::new(3, DistanceFunction::Euclidean, config)
    }

    #[test]
    fn test_insert_and_get() {
        let index = create_test_index();

        index.insert("vec1", &[1.0, 2.0, 3.0]).unwrap();
        index.insert("vec2", &[4.0, 5.0, 6.0]).unwrap();

        assert_eq!(index.len(), 2);
        assert!(index.contains("vec1"));

        let vec1 = index.get("vec1").unwrap().unwrap();
        assert_eq!(vec1, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_search_basic() {
        let index = create_test_index();

        // Insert some vectors
        index.insert("origin", &[0.0, 0.0, 0.0]).unwrap();
        index.insert("near", &[1.0, 1.0, 1.0]).unwrap();
        index.insert("far", &[10.0, 10.0, 10.0]).unwrap();

        // Search from origin
        let results = index.search(&[0.0, 0.0, 0.0], 3).unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].id, "origin");
        assert!(results[0].distance < 0.001);
    }

    #[test]
    fn test_search_accuracy() {
        let index = create_test_index();

        // Insert vectors at known positions
        for i in 0..20 {
            index
                .insert(&format!("vec{}", i), &[i as f32, 0.0, 0.0])
                .unwrap();
        }

        // Search near vec5
        let results = index.search(&[5.0, 0.0, 0.0], 3).unwrap();

        // Should find vec5 as closest
        assert_eq!(results[0].id, "vec5");

        // vec4 and vec6 should be next closest
        let ids: Vec<_> = results.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"vec4") || ids.contains(&"vec6"));
    }

    #[test]
    fn test_delete() {
        let index = create_test_index();

        index.insert("vec1", &[1.0, 2.0, 3.0]).unwrap();
        index.insert("vec2", &[4.0, 5.0, 6.0]).unwrap();

        assert!(index.delete("vec1").unwrap());
        assert!(!index.contains("vec1"));
        assert_eq!(index.len(), 1);

        // Search should not return deleted vector
        let results = index.search(&[1.0, 2.0, 3.0], 5).unwrap();
        assert!(results.iter().all(|r| r.id != "vec1"));
    }

    #[test]
    fn test_empty_index() {
        let index = create_test_index();

        let results = index.search(&[1.0, 2.0, 3.0], 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_dimension_mismatch() {
        let index = create_test_index();

        let result = index.insert("vec1", &[1.0, 2.0]); // Wrong dimensions
        assert!(result.is_err());
    }

    #[test]
    fn test_larger_dataset() {
        let config = HnswConfig::default();
        let index = HnswIndex::new(32, DistanceFunction::Euclidean, config);

        // Insert 100 random-ish vectors
        for i in 0..100 {
            let vec: Vec<f32> = (0..32).map(|j| (i * j) as f32 % 100.0).collect();
            index.insert(&format!("vec{}", i), &vec).unwrap();
        }

        assert_eq!(index.len(), 100);

        // Search should return k results
        let query: Vec<f32> = (0..32).map(|j| (50 * j) as f32 % 100.0).collect();
        let results = index.search(&query, 10).unwrap();
        assert_eq!(results.len(), 10);

        // Results should be sorted by distance
        for i in 1..results.len() {
            assert!(results[i - 1].distance <= results[i].distance);
        }
    }
}
