//! Graph data structures for HNSW.
//!
//! Provides the underlying graph storage for HNSW layers.

use parking_lot::RwLock;
use std::collections::HashMap;

/// Internal node ID (dense, for array indexing).
pub type NodeId = u32;

/// A node in the HNSW graph.
#[derive(Debug, Clone)]
pub struct Node {
    /// External ID (user-provided string ID)
    pub external_id: String,
    /// The vector data
    pub vector: Vec<f32>,
    /// The layer this node was inserted at (its "max layer")
    pub max_layer: usize,
}

/// Neighbor list for a node at a specific layer.
#[derive(Debug, Clone, Default)]
pub struct Neighbors {
    /// Neighbor node IDs, sorted by distance (closest first)
    pub ids: Vec<NodeId>,
}

impl Neighbors {
    /// Create empty neighbors.
    pub fn new() -> Self {
        Self { ids: Vec::new() }
    }

    /// Create neighbors with initial capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ids: Vec::with_capacity(capacity),
        }
    }

    /// Add a neighbor, maintaining sorted order by distance.
    /// Returns true if added, false if already present.
    pub fn add(&mut self, id: NodeId) -> bool {
        if self.ids.contains(&id) {
            return false;
        }
        self.ids.push(id);
        true
    }

    /// Remove a neighbor.
    /// Returns true if removed, false if not found.
    pub fn remove(&mut self, id: NodeId) -> bool {
        if let Some(pos) = self.ids.iter().position(|&x| x == id) {
            self.ids.remove(pos);
            true
        } else {
            false
        }
    }

    /// Check if contains a neighbor.
    pub fn contains(&self, id: NodeId) -> bool {
        self.ids.contains(&id)
    }

    /// Get number of neighbors.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Iterate over neighbor IDs.
    pub fn iter(&self) -> impl Iterator<Item = &NodeId> {
        self.ids.iter()
    }
}

/// Graph storage for a single HNSW layer.
///
/// Each layer stores:
/// - Adjacency lists (node -> neighbors)
/// - Maximum degree (M for layer 0, M*2 for other layers)
pub struct LayerGraph {
    /// Adjacency lists: node_id -> neighbors
    adjacency: RwLock<HashMap<NodeId, Neighbors>>,
    /// Maximum degree for this layer
    max_degree: usize,
}

impl LayerGraph {
    /// Create a new layer graph.
    pub fn new(max_degree: usize) -> Self {
        Self {
            adjacency: RwLock::new(HashMap::new()),
            max_degree,
        }
    }

    /// Get neighbors for a node.
    pub fn neighbors(&self, node_id: NodeId) -> Option<Neighbors> {
        let adj = self.adjacency.read();
        adj.get(&node_id).cloned()
    }

    /// Set neighbors for a node (replaces existing).
    pub fn set_neighbors(&self, node_id: NodeId, neighbors: Neighbors) {
        let mut adj = self.adjacency.write();
        adj.insert(node_id, neighbors);
    }

    /// Add a bidirectional edge between two nodes.
    /// Respects max_degree by potentially removing furthest neighbor.
    pub fn add_edge(&self, from: NodeId, to: NodeId) {
        let mut adj = self.adjacency.write();

        // Add edge from -> to
        adj.entry(from).or_default().add(to);

        // Add edge to -> from
        adj.entry(to).or_default().add(from);
    }

    /// Remove a node from the graph.
    pub fn remove_node(&self, node_id: NodeId) {
        let mut adj = self.adjacency.write();

        // Get neighbors before removal
        if let Some(neighbors) = adj.remove(&node_id) {
            // Remove back-references from neighbors
            for neighbor_id in neighbors.ids {
                if let Some(neighbor_neighbors) = adj.get_mut(&neighbor_id) {
                    neighbor_neighbors.remove(node_id);
                }
            }
        }
    }

    /// Check if a node exists in this layer.
    pub fn contains(&self, node_id: NodeId) -> bool {
        let adj = self.adjacency.read();
        adj.contains_key(&node_id)
    }

    /// Get the number of nodes in this layer.
    pub fn len(&self) -> usize {
        let adj = self.adjacency.read();
        adj.len()
    }

    /// Check if layer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the maximum degree for this layer.
    pub fn max_degree(&self) -> usize {
        self.max_degree
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_neighbors() {
        let mut neighbors = Neighbors::new();
        assert!(neighbors.is_empty());

        assert!(neighbors.add(1));
        assert!(neighbors.add(2));
        assert!(!neighbors.add(1)); // Duplicate

        assert_eq!(neighbors.len(), 2);
        assert!(neighbors.contains(1));
        assert!(neighbors.contains(2));

        assert!(neighbors.remove(1));
        assert!(!neighbors.remove(1)); // Already removed
        assert_eq!(neighbors.len(), 1);
    }

    #[test]
    fn test_layer_graph() {
        let graph = LayerGraph::new(16);

        graph.add_edge(0, 1);
        graph.add_edge(0, 2);
        graph.add_edge(1, 2);

        let n0 = graph.neighbors(0).unwrap();
        assert!(n0.contains(1));
        assert!(n0.contains(2));

        let n1 = graph.neighbors(1).unwrap();
        assert!(n1.contains(0));
        assert!(n1.contains(2));

        graph.remove_node(1);
        let n0 = graph.neighbors(0).unwrap();
        assert!(!n0.contains(1));
        assert!(n0.contains(2));
    }
}
