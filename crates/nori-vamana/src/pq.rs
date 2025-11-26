//! Product Quantization for memory-efficient vector compression.
//!
//! Product Quantization (PQ) compresses high-dimensional vectors into compact
//! codes while enabling fast approximate distance computation.
//!
//! # How It Works
//!
//! 1. **Split**: Divide vector into M subspaces (e.g., 128D → 16 x 8D)
//! 2. **Cluster**: K-means on each subspace to find K centroids (typically 256)
//! 3. **Encode**: Each subvector → nearest centroid ID (8 bits if K=256)
//! 4. **Distance**: Precompute query-to-centroid distances, lookup by code
//!
//! # Compression Ratio
//!
//! For 128D float32 vectors with M=16 subspaces, K=256 centroids:
//! - Original: 128 * 4 = 512 bytes
//! - Compressed: 16 * 1 = 16 bytes
//! - Ratio: 32x
//!
//! # Example
//!
//! ```ignore
//! use nori_vamana::{ProductQuantizer, PQConfig};
//!
//! // Train PQ on a sample of vectors
//! let config = PQConfig { num_subspaces: 16, bits_per_code: 8 };
//! let pq = ProductQuantizer::train(&training_vectors, config)?;
//!
//! // Encode a vector
//! let code = pq.encode(&vector);
//!
//! // Compute approximate distance
//! let dist = pq.asymmetric_distance(&query, &code);
//! ```

use crate::Result;
use rand::seq::SliceRandom;

/// Product Quantization configuration.
#[derive(Debug, Clone)]
pub struct PQConfig {
    /// Number of subspaces (M). Default: 16.
    /// Vector dimension must be divisible by this.
    pub num_subspaces: usize,

    /// Bits per code (determines K = 2^bits). Default: 8 (K=256).
    pub bits_per_code: usize,

    /// Number of k-means iterations for training. Default: 25.
    pub kmeans_iterations: usize,

    /// Sample size for training. Default: 10000.
    pub training_sample_size: usize,
}

impl Default for PQConfig {
    fn default() -> Self {
        Self {
            num_subspaces: 16,
            bits_per_code: 8,
            kmeans_iterations: 25,
            training_sample_size: 10000,
        }
    }
}

/// Product Quantizer.
///
/// Compresses vectors into compact codes for memory-efficient storage
/// and fast approximate distance computation.
pub struct ProductQuantizer {
    /// Number of subspaces
    num_subspaces: usize,

    /// Dimension of each subspace
    subspace_dim: usize,

    /// Number of centroids per subspace (K = 2^bits_per_code)
    num_centroids: usize,

    /// Codebook: [subspace][centroid] -> subvector
    /// Shape: [num_subspaces][num_centroids][subspace_dim]
    codebooks: Vec<Vec<Vec<f32>>>,

    /// Total vector dimensions
    dimensions: usize,
}

impl ProductQuantizer {
    /// Train a Product Quantizer on a sample of vectors.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Training vectors
    /// * `dimensions` - Vector dimensions
    /// * `config` - PQ configuration
    pub fn train(
        vectors: &[Vec<f32>],
        dimensions: usize,
        config: PQConfig,
    ) -> Result<Self> {
        let num_subspaces = config.num_subspaces;
        let num_centroids = 1 << config.bits_per_code; // 2^bits
        let subspace_dim = dimensions / num_subspaces;

        if dimensions % num_subspaces != 0 {
            return Err(crate::VamanaError::ProductQuantization(format!(
                "Dimensions {} not divisible by num_subspaces {}",
                dimensions, num_subspaces
            )));
        }

        // Sample training vectors if needed
        let training_vectors: Vec<_> = if vectors.len() > config.training_sample_size {
            let mut rng = rand::thread_rng();
            let mut indices: Vec<_> = (0..vectors.len()).collect();
            indices.shuffle(&mut rng);
            indices
                .into_iter()
                .take(config.training_sample_size)
                .map(|i| &vectors[i])
                .collect()
        } else {
            vectors.iter().collect()
        };

        // Train codebook for each subspace
        let mut codebooks = Vec::with_capacity(num_subspaces);

        for subspace_idx in 0..num_subspaces {
            let start = subspace_idx * subspace_dim;
            let end = start + subspace_dim;

            // Extract subvectors for this subspace
            let subvectors: Vec<Vec<f32>> = training_vectors
                .iter()
                .map(|v| v[start..end].to_vec())
                .collect();

            // Run k-means to find centroids
            let centroids = kmeans(
                &subvectors,
                num_centroids,
                config.kmeans_iterations,
            );

            codebooks.push(centroids);
        }

        Ok(Self {
            num_subspaces,
            subspace_dim,
            num_centroids,
            codebooks,
            dimensions,
        })
    }

    /// Encode a vector to PQ codes.
    ///
    /// Returns a vector of centroid indices, one per subspace.
    pub fn encode(&self, vector: &[f32]) -> Vec<u8> {
        debug_assert_eq!(vector.len(), self.dimensions);

        let mut codes = Vec::with_capacity(self.num_subspaces);

        for subspace_idx in 0..self.num_subspaces {
            let start = subspace_idx * self.subspace_dim;
            let end = start + self.subspace_dim;
            let subvector = &vector[start..end];

            // Find nearest centroid
            let centroid_idx = self.find_nearest_centroid(subspace_idx, subvector);
            codes.push(centroid_idx as u8);
        }

        codes
    }

    /// Find the nearest centroid in a subspace.
    fn find_nearest_centroid(&self, subspace_idx: usize, subvector: &[f32]) -> usize {
        let centroids = &self.codebooks[subspace_idx];
        let mut best_idx = 0;
        let mut best_dist = f32::MAX;

        for (idx, centroid) in centroids.iter().enumerate() {
            let dist = squared_euclidean(subvector, centroid);
            if dist < best_dist {
                best_dist = dist;
                best_idx = idx;
            }
        }

        best_idx
    }

    /// Compute asymmetric distance between a query (full) and code (compressed).
    ///
    /// This precomputes query-to-centroid distances for fast lookup.
    pub fn asymmetric_distance(&self, query: &[f32], code: &[u8]) -> f32 {
        debug_assert_eq!(query.len(), self.dimensions);
        debug_assert_eq!(code.len(), self.num_subspaces);

        let mut total_dist = 0.0f32;

        for subspace_idx in 0..self.num_subspaces {
            let start = subspace_idx * self.subspace_dim;
            let end = start + self.subspace_dim;
            let query_subvector = &query[start..end];

            let centroid_idx = code[subspace_idx] as usize;
            let centroid = &self.codebooks[subspace_idx][centroid_idx];

            total_dist += squared_euclidean(query_subvector, centroid);
        }

        total_dist.sqrt()
    }

    /// Precompute distance table for a query.
    ///
    /// Returns a table where table[subspace][centroid] = distance.
    /// This enables O(M) distance computation per code.
    pub fn precompute_distance_table(&self, query: &[f32]) -> Vec<Vec<f32>> {
        let mut table = Vec::with_capacity(self.num_subspaces);

        for subspace_idx in 0..self.num_subspaces {
            let start = subspace_idx * self.subspace_dim;
            let end = start + self.subspace_dim;
            let query_subvector = &query[start..end];

            let distances: Vec<f32> = self.codebooks[subspace_idx]
                .iter()
                .map(|centroid| squared_euclidean(query_subvector, centroid))
                .collect();

            table.push(distances);
        }

        table
    }

    /// Compute distance using precomputed table.
    pub fn distance_from_table(&self, table: &[Vec<f32>], code: &[u8]) -> f32 {
        let mut total_dist = 0.0f32;

        for (subspace_idx, &centroid_idx) in code.iter().enumerate() {
            total_dist += table[subspace_idx][centroid_idx as usize];
        }

        total_dist.sqrt()
    }

    /// Get the number of subspaces.
    pub fn num_subspaces(&self) -> usize {
        self.num_subspaces
    }

    /// Get the number of centroids per subspace.
    pub fn num_centroids(&self) -> usize {
        self.num_centroids
    }

    /// Get the total dimensions.
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// Serialize the PQ codebook to bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Header
        bytes.extend_from_slice(&(self.num_subspaces as u32).to_le_bytes());
        bytes.extend_from_slice(&(self.subspace_dim as u32).to_le_bytes());
        bytes.extend_from_slice(&(self.num_centroids as u32).to_le_bytes());
        bytes.extend_from_slice(&(self.dimensions as u32).to_le_bytes());

        // Codebooks
        for subspace_centroids in &self.codebooks {
            for centroid in subspace_centroids {
                for &val in centroid {
                    bytes.extend_from_slice(&val.to_le_bytes());
                }
            }
        }

        bytes
    }

    /// Deserialize PQ codebook from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(crate::VamanaError::ProductQuantization(
                "Invalid PQ bytes: too short".to_string(),
            ));
        }

        let num_subspaces = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let subspace_dim = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        let num_centroids = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        let dimensions = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;

        let mut offset = 16;
        let mut codebooks = Vec::with_capacity(num_subspaces);

        for _ in 0..num_subspaces {
            let mut centroids = Vec::with_capacity(num_centroids);
            for _ in 0..num_centroids {
                let mut centroid = Vec::with_capacity(subspace_dim);
                for _ in 0..subspace_dim {
                    let val = f32::from_le_bytes([
                        bytes[offset],
                        bytes[offset + 1],
                        bytes[offset + 2],
                        bytes[offset + 3],
                    ]);
                    centroid.push(val);
                    offset += 4;
                }
                centroids.push(centroid);
            }
            codebooks.push(centroids);
        }

        Ok(Self {
            num_subspaces,
            subspace_dim,
            num_centroids,
            codebooks,
            dimensions,
        })
    }
}

/// Squared Euclidean distance.
fn squared_euclidean(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum()
}

/// Simple k-means clustering.
fn kmeans(vectors: &[Vec<f32>], k: usize, iterations: usize) -> Vec<Vec<f32>> {
    if vectors.is_empty() || k == 0 {
        return vec![];
    }

    let dim = vectors[0].len();

    // Initialize centroids randomly
    let mut rng = rand::thread_rng();
    let mut indices: Vec<_> = (0..vectors.len()).collect();
    indices.shuffle(&mut rng);

    let mut centroids: Vec<Vec<f32>> = indices
        .into_iter()
        .take(k.min(vectors.len()))
        .map(|i| vectors[i].clone())
        .collect();

    // Pad with zeros if not enough vectors
    while centroids.len() < k {
        centroids.push(vec![0.0; dim]);
    }

    // Iterate
    for _ in 0..iterations {
        // Assign vectors to nearest centroid
        let mut assignments: Vec<Vec<usize>> = vec![vec![]; k];

        for (vec_idx, vector) in vectors.iter().enumerate() {
            let mut best_centroid = 0;
            let mut best_dist = f32::MAX;

            for (centroid_idx, centroid) in centroids.iter().enumerate() {
                let dist = squared_euclidean(vector, centroid);
                if dist < best_dist {
                    best_dist = dist;
                    best_centroid = centroid_idx;
                }
            }

            assignments[best_centroid].push(vec_idx);
        }

        // Update centroids
        for (centroid_idx, assigned_indices) in assignments.iter().enumerate() {
            if assigned_indices.is_empty() {
                continue;
            }

            let mut new_centroid = vec![0.0f32; dim];
            for &vec_idx in assigned_indices {
                for (d, val) in vectors[vec_idx].iter().enumerate() {
                    new_centroid[d] += val;
                }
            }

            let count = assigned_indices.len() as f32;
            for val in &mut new_centroid {
                *val /= count;
            }

            centroids[centroid_idx] = new_centroid;
        }
    }

    centroids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_test_vectors(n: usize, dims: usize) -> Vec<Vec<f32>> {
        (0..n)
            .map(|i| (0..dims).map(|j| ((i * j) % 100) as f32 / 100.0).collect())
            .collect()
    }

    #[test]
    fn test_pq_train_and_encode() {
        let vectors = generate_test_vectors(1000, 128);
        let config = PQConfig {
            num_subspaces: 16,
            bits_per_code: 8,
            kmeans_iterations: 10,
            training_sample_size: 500,
        };

        let pq = ProductQuantizer::train(&vectors, 128, config).unwrap();

        assert_eq!(pq.num_subspaces(), 16);
        assert_eq!(pq.num_centroids(), 256);
        assert_eq!(pq.dimensions(), 128);

        // Encode a vector
        let code = pq.encode(&vectors[0]);
        assert_eq!(code.len(), 16);
    }

    #[test]
    fn test_pq_asymmetric_distance() {
        let vectors = generate_test_vectors(1000, 128);
        let config = PQConfig {
            num_subspaces: 16,
            bits_per_code: 8,
            kmeans_iterations: 10,
            training_sample_size: 500,
        };

        let pq = ProductQuantizer::train(&vectors, 128, config).unwrap();

        let query = &vectors[0];
        let code = pq.encode(&vectors[1]);

        let dist = pq.asymmetric_distance(query, &code);
        assert!(dist >= 0.0);
    }

    #[test]
    fn test_pq_distance_table() {
        let vectors = generate_test_vectors(1000, 128);
        let config = PQConfig::default();

        let pq = ProductQuantizer::train(&vectors, 128, config).unwrap();

        let query = &vectors[0];
        let table = pq.precompute_distance_table(query);

        let code = pq.encode(&vectors[1]);

        let dist_direct = pq.asymmetric_distance(query, &code);
        let dist_table = pq.distance_from_table(&table, &code);

        assert!((dist_direct - dist_table).abs() < 0.001);
    }

    #[test]
    fn test_pq_serialization() {
        let vectors = generate_test_vectors(500, 64);
        let config = PQConfig {
            num_subspaces: 8,
            bits_per_code: 8,
            kmeans_iterations: 5,
            training_sample_size: 500,
        };

        let pq = ProductQuantizer::train(&vectors, 64, config).unwrap();

        // Serialize and deserialize
        let bytes = pq.to_bytes();
        let pq2 = ProductQuantizer::from_bytes(&bytes).unwrap();

        assert_eq!(pq.num_subspaces(), pq2.num_subspaces());
        assert_eq!(pq.num_centroids(), pq2.num_centroids());
        assert_eq!(pq.dimensions(), pq2.dimensions());

        // Verify same encoding
        let code1 = pq.encode(&vectors[0]);
        let code2 = pq2.encode(&vectors[0]);
        assert_eq!(code1, code2);
    }

    #[test]
    fn test_kmeans_basic() {
        let vectors = vec![
            vec![0.0, 0.0],
            vec![0.1, 0.1],
            vec![10.0, 10.0],
            vec![10.1, 10.1],
        ];

        let centroids = kmeans(&vectors, 2, 10);
        assert_eq!(centroids.len(), 2);

        // One centroid should be near (0, 0), other near (10, 10)
        let c1_near_zero = centroids[0][0] < 5.0;
        let c2_near_zero = centroids[1][0] < 5.0;
        assert_ne!(c1_near_zero, c2_near_zero);
    }
}
