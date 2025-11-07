//! Read-index protocol for linearizable reads without leases.
//!
//! Per Raft paper §8 (Read-only queries):
//! When leader lease is not available or has expired, use read-index:
//!
//! 1. Leader records current commit index as the "read index"
//! 2. Leader sends heartbeat to all followers
//! 3. If majority respond, leader confirms it's still the leader
//! 4. Leader waits for state machine to apply up to read index
//! 5. Leader serves the read
//!
//! This ensures linearizability: the read sees all writes committed before it.
//!
//! # Comparison with Lease-based Reads
//!
//! - **Lease**: Fast (no network), requires clock assumptions
//! - **Read-index**: Slower (1 RTT), no clock assumptions, always safe
//!
//! # Design
//!
//! ```text
//! Client read request → Leader checks lease
//!   ├─ Lease valid → Serve immediately
//!   └─ Lease expired → Read-index protocol:
//!        1. Record read_index = commit_index
//!        2. Send heartbeat to majority
//!        3. Wait for majority ACK
//!        4. Wait for apply to catch up to read_index
//!        5. Serve read
//! ```

use crate::config::RaftConfig;
use crate::error::{RaftError, Result};
use crate::replication::replicate_to_follower;
use crate::state::RaftState;
use crate::transport::RaftTransport;
use crate::types::*;
use std::sync::Arc;
use std::time::Duration;

/// Perform read-index protocol.
///
/// Returns Ok(()) if the leader confirmed it's still leader and applied index caught up.
/// Returns Err if:
/// - Not leader
/// - Failed to get majority acknowledgment (network issues, lost leadership)
/// - Timeout waiting for apply
///
/// This is called when:
/// - Leader lease expired
/// - Leader doesn't support leases
/// - Client explicitly requests read-index
pub async fn read_index(
    state: Arc<RaftState>,
    config: &RaftConfig,
    transport: Arc<dyn RaftTransport>,
) -> Result<()> {
    // 1. Check we're leader
    if state.role() != Role::Leader {
        return Err(RaftError::NotLeader {
            leader: state.leader(),
        });
    }

    // 2. Record current commit index as the read index
    let read_idx = state.commit_index();

    // 3. Send heartbeat to all followers (empty AppendEntries)
    // This confirms we're still the leader
    let followers = {
        let volatile = state.volatile_state().read();
        let all_nodes = volatile.config.all_nodes();
        all_nodes
            .into_iter()
            .filter(|node| node != state.node_id())
            .collect::<Vec<_>>()
    };

    // Send heartbeats in parallel
    let mut heartbeat_futures = Vec::new();

    for follower in &followers {
        let state_clone = state.clone();
        let transport_clone = transport.clone();
        let follower_clone = follower.clone();

        let fut = async move {
            replicate_to_follower(state_clone, &follower_clone, transport_clone).await
        };

        heartbeat_futures.push(fut);
    }

    // Wait for responses
    let results = futures::future::join_all(heartbeat_futures).await;

    // Count successful responses
    let mut success_count = 1; // Leader counts as 1
    for result in results {
        if matches!(result, Ok(true)) {
            success_count += 1;
        }
    }

    // Check if we have majority
    let total_nodes = followers.len() + 1; // +1 for leader
    let quorum = (total_nodes / 2) + 1;

    if success_count < quorum {
        return Err(RaftError::NotLeader {
            leader: state.leader(),
        });
    }

    // 4. Wait for state machine to apply up to read_index
    // Poll last_applied until it catches up or timeout
    let timeout = config.election_timeout_min;
    let start = std::time::Instant::now();

    loop {
        let last_applied = {
            let volatile = state.volatile_state().read();
            volatile.last_applied
        };

        if last_applied >= read_idx {
            // State machine has caught up
            return Ok(());
        }

        if start.elapsed() > timeout {
            return Err(RaftError::Internal {
                reason: "Timeout waiting for state machine to apply".to_string(),
            });
        }

        // Sleep briefly before checking again
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::RaftLog;
    use crate::transport::InMemoryTransport;
    use std::collections::HashMap;
    use tempfile::TempDir;

    async fn create_test_state() -> (Arc<RaftState>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let (log, _) = RaftLog::open(temp_dir.path()).await.unwrap();

        let config = RaftConfig::default();
        let transport: Arc<dyn RaftTransport> = Arc::new(InMemoryTransport::new(
            NodeId::new("n1"),
            HashMap::new(),
        ));

        let initial_config = ConfigEntry::Single(vec![
            NodeId::new("n1"),
            NodeId::new("n2"),
            NodeId::new("n3"),
        ]);

        let state = Arc::new(RaftState::new(
            NodeId::new("n1"),
            config,
            log,
            transport,
            initial_config,
        ));
        (state, temp_dir)
    }

    #[tokio::test]
    async fn test_read_index_not_leader() {
        let (state, _temp) = create_test_state().await;

        let config = RaftConfig::default();
        let transport: Arc<dyn RaftTransport> = Arc::new(InMemoryTransport::new(
            NodeId::new("n1"),
            HashMap::new(),
        ));

        // State is follower (not leader)
        let result = read_index(state, &config, transport).await;

        assert!(matches!(result, Err(RaftError::NotLeader { .. })));
    }

    #[tokio::test]
    async fn test_read_index_as_leader_single_node() {
        let (state, _temp) = create_test_state().await;

        // Make single-node cluster
        {
            let mut volatile = state.volatile_state().write();
            volatile.config = ConfigEntry::Single(vec![NodeId::new("n1")]);
        }

        // Set term and become leader
        state.set_current_term(Term(1));
        state.become_leader().await.unwrap();

        let config = RaftConfig::default();
        let transport: Arc<dyn RaftTransport> = Arc::new(InMemoryTransport::new(
            NodeId::new("n1"),
            HashMap::new(),
        ));

        // Should succeed immediately (single node = majority)
        let result = read_index(state, &config, transport).await;
        assert!(result.is_ok());
    }
}
