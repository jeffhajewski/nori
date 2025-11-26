//! Distance functions for vector similarity.
//!
//! Provides three common distance/similarity metrics:
//! - **Euclidean (L2)**: Traditional distance, good for general use
//! - **Cosine**: Angle-based similarity, good for normalized embeddings
//! - **Inner Product**: Dot product, good for maximum inner product search (MIPS)
//!
//! All functions are designed to be auto-vectorized by the compiler when using
//! release builds with appropriate target features.

/// Distance function enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DistanceFunction {
    /// Euclidean (L2) distance: sqrt(sum((a[i] - b[i])^2))
    Euclidean,
    /// Cosine distance: 1 - (a · b) / (||a|| * ||b||)
    Cosine,
    /// Inner product (negative for distance): -(a · b)
    /// Lower is better (more similar), so we negate the dot product.
    InnerProduct,
}

impl DistanceFunction {
    /// Compute distance between two vectors.
    ///
    /// Returns a distance value where lower = more similar.
    #[inline]
    pub fn distance(&self, a: &[f32], b: &[f32]) -> f32 {
        match self {
            Self::Euclidean => euclidean_distance(a, b),
            Self::Cosine => cosine_distance(a, b),
            Self::InnerProduct => -inner_product(a, b),
        }
    }
}

/// Compute Euclidean (L2) distance between two vectors.
///
/// This is the "straight-line" distance in n-dimensional space.
/// Returns sqrt(sum((a[i] - b[i])^2)).
///
/// # Performance
///
/// This function is designed to be auto-vectorized. For best performance,
/// compile with `-C target-cpu=native` or enable specific SIMD features.
///
/// # Example
///
/// ```
/// use nori_vector::euclidean_distance;
///
/// let a = [1.0, 2.0, 3.0];
/// let b = [4.0, 5.0, 6.0];
/// let dist = euclidean_distance(&a, &b);
/// assert!((dist - 5.196).abs() < 0.01); // sqrt(27)
/// ```
#[inline]
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let sum_sq = euclidean_distance_squared(a, b);
    sum_sq.sqrt()
}

/// Compute squared Euclidean distance (avoids sqrt for comparisons).
///
/// For k-NN search, we often only need relative ordering, so squared
/// distance is sufficient and faster (no sqrt).
#[inline]
pub fn euclidean_distance_squared(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    // Process in chunks of 4 for better auto-vectorization
    let mut sum = 0.0f32;
    let chunks = a.len() / 4;

    for i in 0..chunks {
        let base = i * 4;
        let d0 = a[base] - b[base];
        let d1 = a[base + 1] - b[base + 1];
        let d2 = a[base + 2] - b[base + 2];
        let d3 = a[base + 3] - b[base + 3];
        sum += d0 * d0 + d1 * d1 + d2 * d2 + d3 * d3;
    }

    // Handle remaining elements
    for i in (chunks * 4)..a.len() {
        let d = a[i] - b[i];
        sum += d * d;
    }

    sum
}

/// Compute cosine distance between two vectors.
///
/// Cosine distance = 1 - cosine_similarity
/// Where cosine_similarity = (a · b) / (||a|| * ||b||)
///
/// Returns a value in [0, 2] where:
/// - 0 = identical direction
/// - 1 = orthogonal
/// - 2 = opposite direction
///
/// # Example
///
/// ```
/// use nori_vector::cosine_distance;
///
/// let a = [1.0, 0.0];
/// let b = [1.0, 0.0];
/// assert!((cosine_distance(&a, &b) - 0.0).abs() < 0.001);
///
/// let c = [1.0, 0.0];
/// let d = [0.0, 1.0];
/// assert!((cosine_distance(&c, &d) - 1.0).abs() < 0.001);
/// ```
#[inline]
pub fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let (dot, norm_a, norm_b) = dot_and_norms(a, b);

    let denom = (norm_a * norm_b).sqrt();
    if denom < f32::EPSILON {
        return 1.0; // Undefined for zero vectors, return orthogonal
    }

    let similarity = dot / denom;
    // Clamp to [-1, 1] to handle floating point errors
    let similarity = similarity.clamp(-1.0, 1.0);

    1.0 - similarity
}

/// Compute inner product (dot product) of two vectors.
///
/// Returns a · b = sum(a[i] * b[i])
///
/// Higher values indicate more similarity. For use as a distance function,
/// negate the result (lower = more similar).
///
/// # Example
///
/// ```
/// use nori_vector::inner_product;
///
/// let a = [1.0, 2.0, 3.0];
/// let b = [4.0, 5.0, 6.0];
/// let ip = inner_product(&a, &b);
/// assert!((ip - 32.0).abs() < 0.001); // 1*4 + 2*5 + 3*6 = 32
/// ```
#[inline]
pub fn inner_product(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    // Process in chunks of 4 for better auto-vectorization
    let mut sum = 0.0f32;
    let chunks = a.len() / 4;

    for i in 0..chunks {
        let base = i * 4;
        sum += a[base] * b[base]
            + a[base + 1] * b[base + 1]
            + a[base + 2] * b[base + 2]
            + a[base + 3] * b[base + 3];
    }

    // Handle remaining elements
    for i in (chunks * 4)..a.len() {
        sum += a[i] * b[i];
    }

    sum
}

/// Compute dot product and squared norms in a single pass.
///
/// Returns (dot, norm_a_squared, norm_b_squared)
#[inline]
fn dot_and_norms(a: &[f32], b: &[f32]) -> (f32, f32, f32) {
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    let chunks = a.len() / 4;

    for i in 0..chunks {
        let base = i * 4;

        dot += a[base] * b[base]
            + a[base + 1] * b[base + 1]
            + a[base + 2] * b[base + 2]
            + a[base + 3] * b[base + 3];

        norm_a += a[base] * a[base]
            + a[base + 1] * a[base + 1]
            + a[base + 2] * a[base + 2]
            + a[base + 3] * a[base + 3];

        norm_b += b[base] * b[base]
            + b[base + 1] * b[base + 1]
            + b[base + 2] * b[base + 2]
            + b[base + 3] * b[base + 3];
    }

    for i in (chunks * 4)..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    (dot, norm_a, norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_euclidean_distance() {
        let a = [0.0, 0.0, 0.0];
        let b = [3.0, 4.0, 0.0];
        assert!((euclidean_distance(&a, &b) - 5.0).abs() < 0.001);

        // Same vectors = 0 distance
        let c = [1.0, 2.0, 3.0];
        assert!(euclidean_distance(&c, &c) < 0.001);
    }

    #[test]
    fn test_euclidean_distance_squared() {
        let a = [0.0, 0.0];
        let b = [3.0, 4.0];
        assert!((euclidean_distance_squared(&a, &b) - 25.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_distance() {
        // Identical vectors = 0 distance
        let a = [1.0, 2.0, 3.0];
        assert!(cosine_distance(&a, &a) < 0.001);

        // Orthogonal vectors = 1 distance
        let b = [1.0, 0.0];
        let c = [0.0, 1.0];
        assert!((cosine_distance(&b, &c) - 1.0).abs() < 0.001);

        // Opposite vectors = 2 distance
        let d = [1.0, 0.0];
        let e = [-1.0, 0.0];
        assert!((cosine_distance(&d, &e) - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_inner_product() {
        let a = [1.0, 2.0, 3.0];
        let b = [4.0, 5.0, 6.0];
        // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
        assert!((inner_product(&a, &b) - 32.0).abs() < 0.001);
    }

    #[test]
    fn test_distance_function_enum() {
        let a = [1.0, 0.0];
        let b = [0.0, 1.0];

        // Euclidean: sqrt(1 + 1) = sqrt(2)
        let d = DistanceFunction::Euclidean.distance(&a, &b);
        assert!((d - std::f32::consts::SQRT_2).abs() < 0.001);

        // Cosine: orthogonal = 1
        let d = DistanceFunction::Cosine.distance(&a, &b);
        assert!((d - 1.0).abs() < 0.001);

        // Inner product: 0 (orthogonal), negated = 0
        let d = DistanceFunction::InnerProduct.distance(&a, &b);
        assert!(d.abs() < 0.001);
    }

    #[test]
    fn test_high_dimensional() {
        // Test with 128 dimensions (common embedding size)
        let a: Vec<f32> = (0..128).map(|i| i as f32).collect();
        let b: Vec<f32> = (0..128).map(|i| (i + 1) as f32).collect();

        let d = euclidean_distance(&a, &b);
        // Each diff is 1, so sqrt(128) ≈ 11.31
        assert!((d - (128.0f32).sqrt()).abs() < 0.01);
    }

    #[test]
    fn test_zero_vector_cosine() {
        let a = [0.0, 0.0, 0.0];
        let b = [1.0, 2.0, 3.0];
        // Zero vector should return 1.0 (orthogonal)
        assert!((cosine_distance(&a, &b) - 1.0).abs() < 0.001);
    }
}
