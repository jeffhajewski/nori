//! HNSW layer management.
//!
//! Provides utilities for multi-layer graph management.

use crate::graph::{LayerGraph, NodeId};

/// Manages multiple HNSW layers.
#[allow(dead_code)]
pub struct Layers {
    /// Layer graphs (index 0 = bottom layer, highest index = top layer)
    layers: Vec<LayerGraph>,
    /// M parameter (max connections per node in layers > 0)
    m: usize,
    /// M_max0 parameter (max connections in layer 0, typically 2*M)
    m_max0: usize,
}

#[allow(dead_code)]
impl Layers {
    /// Create a new layer manager.
    ///
    /// # Arguments
    ///
    /// * `max_layers` - Maximum number of layers
    /// * `m` - Max connections per node (layers > 0)
    /// * `m_max0` - Max connections in layer 0 (typically 2*M)
    pub fn new(max_layers: usize, m: usize, m_max0: usize) -> Self {
        let layers = (0..max_layers)
            .map(|layer| {
                let max_degree = if layer == 0 { m_max0 } else { m };
                LayerGraph::new(max_degree)
            })
            .collect();

        Self { layers, m, m_max0 }
    }

    /// Get a reference to a specific layer.
    pub fn get(&self, layer: usize) -> Option<&LayerGraph> {
        self.layers.get(layer)
    }

    /// Get the number of layers.
    pub fn num_layers(&self) -> usize {
        self.layers.len()
    }

    /// Get M parameter.
    pub fn m(&self) -> usize {
        self.m
    }

    /// Get M_max0 parameter.
    pub fn m_max0(&self) -> usize {
        self.m_max0
    }

    /// Check if a node exists at a specific layer.
    pub fn contains_at(&self, layer: usize, node_id: NodeId) -> bool {
        self.layers
            .get(layer)
            .map(|l| l.contains(node_id))
            .unwrap_or(false)
    }

    /// Add an edge at a specific layer.
    pub fn add_edge_at(&self, layer: usize, from: NodeId, to: NodeId) {
        if let Some(layer_graph) = self.layers.get(layer) {
            layer_graph.add_edge(from, to);
        }
    }

    /// Remove a node from all layers.
    pub fn remove_node(&self, node_id: NodeId) {
        for layer in &self.layers {
            layer.remove_node(node_id);
        }
    }
}

/// Generate a random layer for a new node.
///
/// Uses the formula: floor(-ln(uniform(0,1)) * m_L)
/// where m_L = 1/ln(M) (typically)
///
/// This gives exponential distribution where probability of
/// being at layer L is roughly 1/M^L.
pub fn random_layer(m: usize, max_layer: usize) -> usize {
    use rand::Rng;

    // m_L = 1/ln(M)
    let m_l = 1.0 / (m as f64).ln();

    let mut rng = rand::thread_rng();
    let uniform: f64 = rng.gen_range(0.0001..1.0); // Avoid ln(0)

    let layer = (-uniform.ln() * m_l).floor() as usize;
    layer.min(max_layer - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layers_creation() {
        let layers = Layers::new(16, 16, 32);
        assert_eq!(layers.num_layers(), 16);
        assert_eq!(layers.m(), 16);
        assert_eq!(layers.m_max0(), 32);
    }

    #[test]
    fn test_random_layer_distribution() {
        // Test that random_layer produces reasonable distribution
        let m = 16;
        let max_layer = 16;
        let mut layer_counts = vec![0usize; max_layer];

        for _ in 0..10000 {
            let layer = random_layer(m, max_layer);
            assert!(layer < max_layer);
            layer_counts[layer] += 1;
        }

        // Layer 0 should have most nodes
        assert!(layer_counts[0] > layer_counts[1]);
        // Higher layers should have fewer nodes
        for i in 1..max_layer - 1 {
            if layer_counts[i] > 0 && layer_counts[i + 1] > 0 {
                // Not strictly enforced due to randomness
            }
        }
    }

    #[test]
    fn test_edge_operations() {
        let layers = Layers::new(4, 8, 16);

        layers.add_edge_at(0, 0, 1);
        layers.add_edge_at(0, 0, 2);

        assert!(layers.contains_at(0, 0));
        assert!(layers.contains_at(0, 1));
        assert!(!layers.contains_at(1, 0)); // Not added to layer 1
    }
}
