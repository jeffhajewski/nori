//! Disk-resident graph storage for Vamana.
//!
//! Stores the Vamana graph on disk using mmap for memory-efficient access.

use crate::Result;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Node ID type (u32 supports ~4 billion vectors).
pub type NodeId = u32;

/// Disk-resident graph.
///
/// The graph is stored with fixed-size adjacency lists for efficient random access.
/// Format:
/// - Header: num_nodes (u32), max_degree (u32)
/// - Adjacency: [node_0_neighbors...][node_1_neighbors...]...
///   - Each node has max_degree slots (u32 each), unused filled with u32::MAX
pub struct DiskGraph {
    /// Memory-mapped file
    #[allow(dead_code)]
    mmap: memmap2::Mmap,

    /// Number of nodes
    num_nodes: usize,

    /// Maximum out-degree per node
    max_degree: usize,

    /// Offset to adjacency data
    adjacency_offset: usize,
}

impl DiskGraph {
    /// Open an existing disk graph.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };

        // Read header
        let num_nodes = u32::from_le_bytes([mmap[0], mmap[1], mmap[2], mmap[3]]) as usize;
        let max_degree = u32::from_le_bytes([mmap[4], mmap[5], mmap[6], mmap[7]]) as usize;

        Ok(Self {
            mmap,
            num_nodes,
            max_degree,
            adjacency_offset: 8,
        })
    }

    /// Get neighbors of a node.
    pub fn neighbors(&self, node_id: NodeId) -> Vec<NodeId> {
        let start = self.adjacency_offset + (node_id as usize) * self.max_degree * 4;

        let mut neighbors = Vec::with_capacity(self.max_degree);
        for i in 0..self.max_degree {
            let offset = start + i * 4;
            let neighbor = u32::from_le_bytes([
                self.mmap[offset],
                self.mmap[offset + 1],
                self.mmap[offset + 2],
                self.mmap[offset + 3],
            ]);

            if neighbor != u32::MAX {
                neighbors.push(neighbor);
            } else {
                break; // End of neighbors
            }
        }

        neighbors
    }

    /// Get the number of nodes.
    pub fn num_nodes(&self) -> usize {
        self.num_nodes
    }

    /// Get the maximum degree.
    pub fn max_degree(&self) -> usize {
        self.max_degree
    }
}

/// Builder for disk graph.
pub struct DiskGraphBuilder {
    /// Adjacency lists during construction
    adjacency: Vec<Vec<NodeId>>,

    /// Maximum degree
    max_degree: usize,
}

impl DiskGraphBuilder {
    /// Create a new graph builder.
    pub fn new(max_degree: usize) -> Self {
        Self {
            adjacency: Vec::new(),
            max_degree,
        }
    }

    /// Add a new node, returns its ID.
    pub fn add_node(&mut self) -> NodeId {
        let id = self.adjacency.len() as NodeId;
        self.adjacency.push(Vec::with_capacity(self.max_degree));
        id
    }

    /// Set neighbors for a node.
    pub fn set_neighbors(&mut self, node_id: NodeId, neighbors: Vec<NodeId>) {
        self.adjacency[node_id as usize] = neighbors;
    }

    /// Get neighbors (during construction).
    pub fn neighbors(&self, node_id: NodeId) -> &[NodeId] {
        &self.adjacency[node_id as usize]
    }

    /// Add an edge (if not over max_degree).
    pub fn add_edge(&mut self, from: NodeId, to: NodeId) -> bool {
        let neighbors = &mut self.adjacency[from as usize];
        if neighbors.len() < self.max_degree && !neighbors.contains(&to) {
            neighbors.push(to);
            true
        } else {
            false
        }
    }

    /// Get number of nodes.
    pub fn num_nodes(&self) -> usize {
        self.adjacency.len()
    }

    /// Write graph to disk.
    pub fn write<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        // Write header
        writer.write_all(&(self.adjacency.len() as u32).to_le_bytes())?;
        writer.write_all(&(self.max_degree as u32).to_le_bytes())?;

        // Write adjacency lists
        for neighbors in &self.adjacency {
            for &neighbor in neighbors {
                writer.write_all(&neighbor.to_le_bytes())?;
            }
            // Pad with u32::MAX
            for _ in neighbors.len()..self.max_degree {
                writer.write_all(&u32::MAX.to_le_bytes())?;
            }
        }

        writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_graph_builder_and_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("graph.bin");

        // Build graph
        let mut builder = DiskGraphBuilder::new(4);
        let n0 = builder.add_node();
        let n1 = builder.add_node();
        let n2 = builder.add_node();

        builder.add_edge(n0, n1);
        builder.add_edge(n0, n2);
        builder.add_edge(n1, n0);
        builder.add_edge(n1, n2);

        builder.write(&path).unwrap();

        // Read graph
        let graph = DiskGraph::open(&path).unwrap();
        assert_eq!(graph.num_nodes(), 3);
        assert_eq!(graph.max_degree(), 4);

        let n0_neighbors = graph.neighbors(0);
        assert_eq!(n0_neighbors.len(), 2);
        assert!(n0_neighbors.contains(&1));
        assert!(n0_neighbors.contains(&2));
    }
}
