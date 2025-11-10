//! Shard placement and consistent hashing for NoriKV.
//!
//! This module implements:
//! - xxhash64 with seed=0 for key hashing
//! - Jump Consistent Hash for minimal-disruption shard assignment
//! - Router trait for request routing (key → shard → node)
//! - Placement trait for replica assignment planning
//!
//! CRITICAL: These implementations must match the client SDKs exactly!

use std::collections::HashMap;
use std::hash::Hasher;
use std::sync::Arc;
use twox_hash::XxHash64;

/// Shard identifier (0..total_shards)
pub type ShardId = u32;

/// Node identifier
pub type NodeId = String;

/// Router trait - routes requests from key → shard → node.
///
/// This is the primary interface for request routing in the server.
pub trait Router: Send + Sync {
    /// Maps a key hash to a shard ID using consistent hashing.
    fn shard_of(&self, key_hash: u64) -> ShardId;

    /// Routes a shard to a leader node.
    ///
    /// Returns None if no leader is known for this shard (during election).
    fn route(&self, shard: ShardId) -> Option<NodeId>;

    /// Gets all replica nodes for a shard (leader + followers).
    fn replicas(&self, shard: ShardId) -> Vec<NodeId>;
}

/// ClusterView - snapshot of cluster topology.
///
/// This represents the current view of the cluster: which nodes are alive,
/// and which shards they are responsible for.
#[derive(Debug, Clone)]
pub struct ClusterView {
    /// Map of shard_id → leader node
    pub leaders: HashMap<ShardId, NodeId>,

    /// Map of shard_id → list of replica nodes (including leader)
    pub replicas: HashMap<ShardId, Vec<NodeId>>,

    /// Set of all live nodes in the cluster
    pub live_nodes: Vec<NodeId>,
}

impl ClusterView {
    /// Creates an empty cluster view.
    pub fn empty() -> Self {
        Self {
            leaders: HashMap::new(),
            replicas: HashMap::new(),
            live_nodes: Vec::new(),
        }
    }

    /// Creates a single-node cluster view (for testing/development).
    pub fn single_node(node_id: NodeId, total_shards: u32) -> Self {
        let mut leaders = HashMap::new();
        let mut replicas = HashMap::new();

        for shard_id in 0..total_shards {
            leaders.insert(shard_id, node_id.clone());
            replicas.insert(shard_id, vec![node_id.clone()]);
        }

        Self {
            leaders,
            replicas,
            live_nodes: vec![node_id],
        }
    }
}

/// Shard assignment plan - which nodes should own which shards.
///
/// This is produced by the Placement trait's plan() method.
#[derive(Debug, Clone)]
pub struct Assignment {
    /// Map of shard_id → ordered list of replica nodes (first is primary)
    pub assignments: HashMap<ShardId, Vec<NodeId>>,
}

impl Assignment {
    /// Creates an empty assignment.
    pub fn empty() -> Self {
        Self {
            assignments: HashMap::new(),
        }
    }
}

/// Move plan - describes which shards need to move when topology changes.
///
/// This is produced by the Placement trait's diff() method.
#[derive(Debug, Clone)]
pub struct MovePlan {
    /// Shards that need to add replicas: (shard_id, new_node)
    pub additions: Vec<(ShardId, NodeId)>,

    /// Shards that need to remove replicas: (shard_id, old_node)
    pub removals: Vec<(ShardId, NodeId)>,
}

impl MovePlan {
    /// Creates an empty move plan.
    pub fn empty() -> Self {
        Self {
            additions: Vec::new(),
            removals: Vec::new(),
        }
    }

    /// Returns true if there are no moves.
    pub fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.removals.is_empty()
    }
}

/// Placement trait - plans shard replica assignments.
///
/// This is used to determine which nodes should own which shards
/// when the cluster topology changes.
pub trait Placement: Send + Sync {
    /// Creates a new shard assignment plan given current cluster view.
    fn plan(&self, view: &ClusterView, total_shards: u32, replication_factor: usize) -> Assignment;

    /// Computes the minimal set of moves needed to go from old to new assignment.
    fn diff(&self, old: &Assignment, new: &Assignment) -> MovePlan;
}

/// Simple router implementation backed by a ClusterView.
///
/// This is the default Router implementation used by the server.
pub struct SimpleRouter {
    /// Total number of virtual shards
    total_shards: u32,

    /// Current cluster view (updated via set_view)
    view: Arc<parking_lot::RwLock<ClusterView>>,
}

impl SimpleRouter {
    /// Creates a new SimpleRouter with the given total shards.
    pub fn new(total_shards: u32) -> Self {
        Self {
            total_shards,
            view: Arc::new(parking_lot::RwLock::new(ClusterView::empty())),
        }
    }

    /// Creates a SimpleRouter with a single-node view (for testing).
    pub fn single_node(node_id: NodeId, total_shards: u32) -> Self {
        Self {
            total_shards,
            view: Arc::new(parking_lot::RwLock::new(ClusterView::single_node(
                node_id,
                total_shards,
            ))),
        }
    }

    /// Updates the cluster view.
    pub fn set_view(&self, view: ClusterView) {
        *self.view.write() = view;
    }

    /// Gets a copy of the current cluster view.
    pub fn get_view(&self) -> ClusterView {
        self.view.read().clone()
    }
}

impl Router for SimpleRouter {
    fn shard_of(&self, key_hash: u64) -> ShardId {
        jump_consistent_hash(key_hash, self.total_shards)
    }

    fn route(&self, shard: ShardId) -> Option<NodeId> {
        self.view.read().leaders.get(&shard).cloned()
    }

    fn replicas(&self, shard: ShardId) -> Vec<NodeId> {
        self.view
            .read()
            .replicas
            .get(&shard)
            .cloned()
            .unwrap_or_default()
    }
}

/// Compute xxhash64 of a key with seed=0.
///
/// This MUST produce identical results to the TypeScript SDK implementation.
///
/// # Examples
///
/// ```
/// use norikv_placement::xxhash64;
///
/// let hash = xxhash64(b"hello");
/// assert_eq!(hash, xxhash64(b"hello")); // Deterministic
/// ```
pub fn xxhash64(key: &[u8]) -> u64 {
    let mut hasher = XxHash64::with_seed(0);
    hasher.write(key);
    hasher.finish()
}

/// Jump Consistent Hash - minimal-disruption consistent hashing.
///
/// Maps a 64-bit hash to a bucket in [0, num_buckets).
/// When num_buckets changes, only ~(1/num_buckets) keys move.
///
/// Reference: https://arxiv.org/abs/1406.2294
///
/// This MUST match the TypeScript implementation exactly!
///
/// # Algorithm
///
/// ```text
/// let mut b = -1;
/// let mut j = 0;
/// while j < num_buckets {
///     b = j;
///     key = key * 2862933555777941757 + 1;
///     j = (b + 1) * (2^31 / ((key >> 33) + 1))
/// }
/// return b;
/// ```
///
/// # Examples
///
/// ```
/// use norikv_placement::jump_consistent_hash;
///
/// let hash = 12345678901234567890u64;
/// let shard = jump_consistent_hash(hash, 1024);
/// assert!(shard < 1024);
/// ```
pub fn jump_consistent_hash(mut key: u64, num_buckets: u32) -> u32 {
    let mut b: i64 = -1;
    let mut j: i64 = 0;

    while j < num_buckets as i64 {
        b = j;
        key = key.wrapping_mul(2862933555777941757).wrapping_add(1);
        j = ((b + 1) as f64 * (f64::from(1u32 << 31) / ((key >> 33) + 1) as f64)) as i64;
    }

    b as u32
}

/// Compute the shard for a given key.
///
/// This combines xxhash64 and jump_consistent_hash to route a key to a shard.
///
/// # Examples
///
/// ```
/// use norikv_placement::get_shard_for_key;
///
/// let shard = get_shard_for_key(b"user:12345", 1024);
/// assert!(shard < 1024);
/// ```
pub fn get_shard_for_key(key: &[u8], total_shards: u32) -> u32 {
    let hash = xxhash64(key);
    jump_consistent_hash(hash, total_shards)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xxhash64_consistency() {
        let key = b"hello";
        let hash1 = xxhash64(key);
        let hash2 = xxhash64(key);
        assert_eq!(hash1, hash2, "xxhash64 should be deterministic");
    }

    #[test]
    fn test_xxhash64_different_keys() {
        let hash1 = xxhash64(b"hello");
        let hash2 = xxhash64(b"world");
        assert_ne!(hash1, hash2, "Different keys should have different hashes");
    }

    #[test]
    fn test_jump_consistent_hash_in_range() {
        let hash = 12345678901234567890u64;
        let num_buckets = 1024;
        let bucket = jump_consistent_hash(hash, num_buckets);
        assert!(bucket < num_buckets, "Bucket should be in valid range");
    }

    #[test]
    fn test_jump_consistent_hash_consistency() {
        let hash = 12345678901234567890u64;
        let num_buckets = 1024;
        let bucket1 = jump_consistent_hash(hash, num_buckets);
        let bucket2 = jump_consistent_hash(hash, num_buckets);
        assert_eq!(bucket1, bucket2, "Jump hash should be deterministic");
    }

    #[test]
    fn test_jump_consistent_hash_distribution() {
        use std::collections::HashSet;
        let mut buckets = HashSet::new();
        let num_buckets = 100;

        for i in 0u64..1000 {
            let hash = xxhash64(&i.to_le_bytes());
            let bucket = jump_consistent_hash(hash, num_buckets);
            buckets.insert(bucket);
        }

        // With 1000 keys and 100 buckets, we should hit most buckets
        assert!(
            buckets.len() > 90,
            "Expected > 90 unique buckets, got {}",
            buckets.len()
        );
    }

    #[test]
    fn test_jump_consistent_hash_minimal_move() {
        // When adding one bucket, only ~1% of keys should move
        let num_keys = 1000;
        let old_buckets = 100;
        let new_buckets = 101;

        let mut moved = 0;
        for i in 0u64..num_keys {
            let hash = xxhash64(&i.to_le_bytes());
            let old_bucket = jump_consistent_hash(hash, old_buckets);
            let new_bucket = jump_consistent_hash(hash, new_buckets);
            if old_bucket != new_bucket {
                moved += 1;
            }
        }

        // Should be roughly 1/101 ≈ 10 keys, allow some variance
        assert!(
            moved > 0 && moved < 20,
            "Expected 1-20 keys to move, got {}",
            moved
        );
    }

    #[test]
    fn test_get_shard_for_key() {
        let shard1 = get_shard_for_key(b"user:123", 1024);
        let shard2 = get_shard_for_key(b"user:123", 1024);
        assert_eq!(shard1, shard2, "Same key should map to same shard");
        assert!(shard1 < 1024, "Shard should be in valid range");
    }

    #[test]
    fn test_cluster_view_empty() {
        let view = ClusterView::empty();
        assert!(view.leaders.is_empty());
        assert!(view.replicas.is_empty());
        assert!(view.live_nodes.is_empty());
    }

    #[test]
    fn test_cluster_view_single_node() {
        let node_id = "node1".to_string();
        let total_shards = 4;
        let view = ClusterView::single_node(node_id.clone(), total_shards);

        assert_eq!(view.live_nodes.len(), 1);
        assert_eq!(view.live_nodes[0], node_id);
        assert_eq!(view.leaders.len(), 4);
        assert_eq!(view.replicas.len(), 4);

        for shard_id in 0..total_shards {
            assert_eq!(view.leaders.get(&shard_id), Some(&node_id));
            assert_eq!(
                view.replicas.get(&shard_id),
                Some(&vec![node_id.clone()])
            );
        }
    }

    #[test]
    fn test_simple_router_shard_of() {
        let router = SimpleRouter::new(1024);
        let hash = xxhash64(b"test-key");
        let shard1 = router.shard_of(hash);
        let shard2 = router.shard_of(hash);

        assert_eq!(shard1, shard2, "shard_of should be deterministic");
        assert!(shard1 < 1024, "Shard should be in valid range");
    }

    #[test]
    fn test_simple_router_route_empty() {
        let router = SimpleRouter::new(1024);
        let result = router.route(0);

        assert_eq!(result, None, "Empty router should return None");
    }

    #[test]
    fn test_simple_router_route_single_node() {
        let node_id = "node1".to_string();
        let router = SimpleRouter::single_node(node_id.clone(), 8);

        for shard_id in 0..8 {
            assert_eq!(
                router.route(shard_id),
                Some(node_id.clone()),
                "Should route to node1 for shard {}",
                shard_id
            );
        }
    }

    #[test]
    fn test_simple_router_replicas() {
        let node_id = "node1".to_string();
        let router = SimpleRouter::single_node(node_id.clone(), 4);

        for shard_id in 0..4 {
            let replicas = router.replicas(shard_id);
            assert_eq!(replicas.len(), 1);
            assert_eq!(replicas[0], node_id);
        }
    }

    #[test]
    fn test_simple_router_set_view() {
        let router = SimpleRouter::new(4);

        // Initially empty
        assert_eq!(router.route(0), None);

        // Set a view
        let mut view = ClusterView::empty();
        view.leaders.insert(0, "node1".to_string());
        view.leaders.insert(1, "node2".to_string());
        router.set_view(view);

        // Should now route correctly
        assert_eq!(router.route(0), Some("node1".to_string()));
        assert_eq!(router.route(1), Some("node2".to_string()));
        assert_eq!(router.route(2), None);
    }

    #[test]
    fn test_assignment_empty() {
        let assignment = Assignment::empty();
        assert!(assignment.assignments.is_empty());
    }

    #[test]
    fn test_move_plan_empty() {
        let plan = MovePlan::empty();
        assert!(plan.is_empty());
        assert!(plan.additions.is_empty());
        assert!(plan.removals.is_empty());
    }

    #[test]
    fn test_move_plan_with_moves() {
        let mut plan = MovePlan::empty();
        plan.additions.push((0, "node1".to_string()));
        plan.removals.push((1, "node2".to_string()));

        assert!(!plan.is_empty());
        assert_eq!(plan.additions.len(), 1);
        assert_eq!(plan.removals.len(), 1);
    }
}
