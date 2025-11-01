//! Log replication for Raft.
//!
//! Per Raft §5.3 (Log Replication):
//! - Leader sends AppendEntries RPCs to replicate log entries
//! - AppendEntries also serves as heartbeat (prevents elections)
//! - Leader tracks next_index and match_index for each follower
//! - Leader commits entries when replicated on majority
//! - Followers apply committed entries to state machine

use crate::config::RaftConfig;
use crate::error::Result;
use crate::state::RaftState;
use crate::transport::RaftTransport;
use crate::types::*;
use std::sync::Arc;
use tokio::time::{interval, Duration};

/// Replicate to a single follower.
///
/// Sends AppendEntries RPC with entries starting from next_index[follower].
/// If follower is too far behind (next_index <= last_snapshot_index),
/// logs a warning that snapshot should be sent.
///
/// Updates next_index and match_index based on response.
///
/// Returns true if replication succeeded, false otherwise.
pub async fn replicate_to_follower(
    state: Arc<RaftState>,
    follower: &NodeId,
    transport: Arc<dyn RaftTransport>,
) -> Result<bool> {
    // Get next_index for this follower and check if behind snapshot
    let (next_idx, last_snapshot_index) = {
        let volatile = state.volatile_state().read();
        let leader_state = volatile
            .leader_state
            .as_ref()
            .ok_or_else(|| crate::error::RaftError::Internal {
                reason: "Not leader".to_string(),
            })?;
        let next_idx = leader_state
            .next_index
            .get(follower)
            .copied()
            .unwrap_or(LogIndex(1));
        (next_idx, volatile.last_snapshot_index)
    };

    // Check if follower needs a snapshot
    // (next_index points to entry that's been compacted)
    if next_idx <= last_snapshot_index && last_snapshot_index > LogIndex::ZERO {
        tracing::warn!(
            follower = ?follower,
            next_index = %next_idx,
            last_snapshot_index = %last_snapshot_index,
            "Follower is behind snapshot point - snapshot transfer not yet implemented. \
             Follower will need to catch up via snapshot installation."
        );
        // TODO: Implement snapshot sending
        // For now, return false to indicate replication did not succeed
        // The follower will remain behind until snapshot support is fully implemented
        return Ok(false);
    }

    // Get prev_log info for consistency check
    let prev_log_index = next_idx.prev().unwrap_or(LogIndex::ZERO);
    let prev_log_term = if prev_log_index == LogIndex::ZERO {
        Term::ZERO
    } else {
        state
            .log_ref()
            .get(prev_log_index)
            .await?
            .map(|e| e.term)
            .unwrap_or(Term::ZERO)
    };

    // Get entries to send (from next_idx to last_index)
    let last_log_index = state.log_ref().last_index().await;
    let entries = if next_idx <= last_log_index {
        state
            .log_ref()
            .get_range(next_idx, last_log_index.next())
            .await?
    } else {
        Vec::new() // Heartbeat (no entries)
    };

    let current_term = state.current_term();
    let leader_commit = state.commit_index();

    // Send AppendEntries RPC
    let request = AppendEntriesRequest {
        term: current_term,
        leader_id: state.node_id().clone(),
        prev_log_index,
        prev_log_term,
        entries: entries.clone(),
        leader_commit,
    };

    match transport.append_entries(follower, request).await {
        Ok(response) => {
            // Check if we're still leader in same term
            if response.term > current_term {
                // Follower has higher term, step down
                return Ok(false);
            }

            let mut volatile = state.volatile_state().write();
            if let Some(leader_state) = volatile.leader_state.as_mut() {
                if response.success {
                    // Update next_index and match_index
                    let new_match_index = if entries.is_empty() {
                        prev_log_index
                    } else {
                        entries.last().unwrap().index
                    };

                    leader_state
                        .next_index
                        .insert(follower.clone(), new_match_index.next());
                    leader_state
                        .match_index
                        .insert(follower.clone(), new_match_index);

                    Ok(true)
                } else {
                    // Log inconsistency, decrement next_index and retry
                    let new_next_index = if let Some(conflict_idx) = response.conflict_index {
                        conflict_idx
                    } else {
                        // No hint, just decrement by 1
                        next_idx.prev().unwrap_or(LogIndex(1))
                    };

                    leader_state
                        .next_index
                        .insert(follower.clone(), new_next_index);

                    Ok(false)
                }
            } else {
                Ok(false) // Not leader anymore
            }
        }
        Err(_) => {
            // Network error, will retry on next heartbeat
            Ok(false)
        }
    }
}

/// Advance commit index based on match_index.
///
/// Per Raft §5.3:
/// If there exists an N such that N > commitIndex, a majority of
/// matchIndex[i] ≥ N, and log[N].term == currentTerm:
/// set commitIndex = N
///
/// Returns true if commit index advanced.
pub async fn advance_commit_index(state: Arc<RaftState>) -> Result<bool> {
    let current_term = state.current_term();
    let current_commit = state.commit_index();
    let last_log_index = state.log_ref().last_index().await;

    // Get all match_index values + our own log
    let match_indices = {
        let volatile = state.volatile_state().read();
        if let Some(leader_state) = &volatile.leader_state {
            let mut indices: Vec<LogIndex> = leader_state.match_index.values().copied().collect();
            indices.push(last_log_index); // Leader's own log
            indices
        } else {
            return Ok(false); // Not leader
        }
    };

    // Find highest N where majority have match_index >= N
    let mut candidate_indices: Vec<LogIndex> = match_indices
        .iter()
        .filter(|&&idx| idx > current_commit)
        .copied()
        .collect();

    if candidate_indices.is_empty() {
        return Ok(false);
    }

    candidate_indices.sort_by(|a, b| b.cmp(a)); // Sort descending

    // Check each candidate index for majority
    let quorum = (match_indices.len() / 2) + 1;

    for candidate_idx in candidate_indices {
        // Count how many replicas have >= candidate_idx
        let count = match_indices
            .iter()
            .filter(|&&idx| idx >= candidate_idx)
            .count();

        if count >= quorum {
            // Check that entry at candidate_idx has current term
            if let Some(entry) = state.log_ref().get(candidate_idx).await? {
                if entry.term == current_term {
                    // Can commit!
                    let mut volatile = state.volatile_state().write();
                    volatile.commit_index = candidate_idx;
                    return Ok(true);
                }
            }
        }
    }

    Ok(false)
}

/// Heartbeat loop for leader.
///
/// Sends AppendEntries (heartbeat or with entries) to all followers
/// at regular intervals (150ms default).
///
/// Also advances commit index when entries are replicated to majority.
pub async fn heartbeat_loop(
    state: Arc<RaftState>,
    config: RaftConfig,
    transport: Arc<dyn RaftTransport>,
    shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    let mut shutdown_rx = shutdown_rx;
    let mut ticker = interval(config.heartbeat_interval);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Only send heartbeats if we're leader
                if state.role() != Role::Leader {
                    continue;
                }

                // Get list of followers
                let followers = {
                    let volatile = state.volatile_state().read();
                    let all_nodes = volatile.config.all_nodes();
                    all_nodes
                        .into_iter()
                        .filter(|node| node != state.node_id())
                        .collect::<Vec<_>>()
                };

                // Replicate to all followers in parallel
                let mut replicate_futures = Vec::new();

                for follower in followers {
                    let state_clone = state.clone();
                    let transport_clone = transport.clone();
                    let follower_clone = follower.clone();

                    let fut = async move {
                        replicate_to_follower(state_clone, &follower_clone, transport_clone).await
                    };

                    replicate_futures.push(fut);
                }

                // Wait for all replication attempts
                let _results = futures::future::join_all(replicate_futures).await;

                // Try to advance commit index
                let _ = advance_commit_index(state.clone()).await;
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("Heartbeat loop shutting down");
                break;
            }
        }
    }
}

/// Apply loop - applies committed entries to state machine.
///
/// Continuously checks if commit_index > last_applied.
/// If so, applies entries [last_applied+1..commit_index] to state machine
/// via the applied channel.
///
/// If a state_machine is provided, entries are automatically applied to it.
/// Otherwise, clients consume from the applied channel to update their state machine.
pub async fn apply_loop(
    state: Arc<RaftState>,
    mut applied_tx: tokio::sync::mpsc::Sender<(LogIndex, bytes::Bytes)>,
    shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    state_machine: Option<Arc<tokio::sync::Mutex<dyn crate::snapshot::StateMachine>>>,
) {
    let mut shutdown_rx = shutdown_rx;
    let mut ticker = interval(Duration::from_millis(10)); // Check every 10ms

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Get last_applied and commit_index
                let (last_applied, commit_index) = {
                    let volatile = state.volatile_state().read();
                    (volatile.last_applied, volatile.commit_index)
                };

                if commit_index > last_applied {
                    // Apply entries from last_applied+1 to commit_index
                    let start_idx = last_applied.next();
                    let end_idx = commit_index.next();

                    match state.log_ref().get_range(start_idx, end_idx).await {
                        Ok(entries) => {
                            for entry in entries {
                                // Apply to state machine if provided
                                if let Some(ref sm) = state_machine {
                                    let mut sm_lock = sm.lock().await;
                                    if let Err(e) = sm_lock.apply(&entry.command) {
                                        tracing::error!(
                                            error = ?e,
                                            index = ?entry.index,
                                            "Failed to apply entry to state machine"
                                        );
                                        // Continue applying other entries
                                    }
                                }

                                // Send to applied channel (for backward compatibility, only if no state machine)
                                if state_machine.is_none() {
                                    if applied_tx.send((entry.index, entry.command.clone())).await.is_err() {
                                        // Receiver dropped, exit
                                        return;
                                    }
                                }

                                // Update last_applied
                                let mut volatile = state.volatile_state().write();
                                volatile.last_applied = entry.index;
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "Failed to read log entries for apply");
                        }
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("Apply loop shutting down");
                break;
            }
        }
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
    async fn test_advance_commit_index() {
        let (state, _temp) = create_test_state().await;

        // Set term to 1 (simulate election)
        state.set_current_term(Term(1));

        // Become leader
        state.become_leader().await.unwrap();

        // Add some entries to log
        use bytes::Bytes;
        for i in 1..=5 {
            let entry = LogEntry::new(Term(1), LogIndex(i), Bytes::from(format!("cmd{}", i)));
            state.log_ref().append(entry).await.unwrap();
        }

        // Simulate match_index for followers
        {
            let mut volatile = state.volatile_state().write();
            if let Some(leader_state) = volatile.leader_state.as_mut() {
                leader_state
                    .match_index
                    .insert(NodeId::new("n2"), LogIndex(3));
                leader_state
                    .match_index
                    .insert(NodeId::new("n3"), LogIndex(3));
            }
        }

        // Try to advance commit index
        let advanced = advance_commit_index(state.clone()).await.unwrap();
        assert!(advanced);

        // Commit index should be 3 (majority have replicated up to 3)
        assert_eq!(state.commit_index(), LogIndex(3));
    }

    #[tokio::test]
    async fn test_advance_commit_index_no_majority() {
        let (state, _temp) = create_test_state().await;

        // Set term to 1 (simulate election)
        state.set_current_term(Term(1));

        // Become leader
        state.become_leader().await.unwrap();

        // Add some entries
        use bytes::Bytes;
        for i in 1..=5 {
            let entry = LogEntry::new(Term(1), LogIndex(i), Bytes::from(format!("cmd{}", i)));
            state.log_ref().append(entry).await.unwrap();
        }

        // Only one follower has replicated to index 1
        // With 3 nodes, we need 2 for quorum. Leader (with 5) + one follower (with 1) = majority at 1
        {
            let mut volatile = state.volatile_state().write();
            if let Some(leader_state) = volatile.leader_state.as_mut() {
                leader_state
                    .match_index
                    .insert(NodeId::new("n2"), LogIndex(1));
                leader_state
                    .match_index
                    .insert(NodeId::new("n3"), LogIndex(0));
            }
        }

        // Try to advance commit index - should advance to 1 (where we have majority)
        let advanced1 = advance_commit_index(state.clone()).await.unwrap();
        assert!(advanced1);
        assert_eq!(state.commit_index(), LogIndex(1));

        // Try to advance again - should NOT advance beyond 1 (no majority for indices > 1)
        let advanced2 = advance_commit_index(state.clone()).await.unwrap();
        assert!(!advanced2);
        assert_eq!(state.commit_index(), LogIndex(1));
    }

    #[tokio::test]
    async fn test_apply_loop() {
        let (state, _temp) = create_test_state().await;

        // Add entries and set commit_index
        use bytes::Bytes;
        for i in 1..=3 {
            let entry = LogEntry::new(Term(1), LogIndex(i), Bytes::from(format!("cmd{}", i)));
            state.log_ref().append(entry).await.unwrap();
        }

        {
            let mut volatile = state.volatile_state().write();
            volatile.commit_index = LogIndex(3);
        }

        // Create channel for applied entries
        let (applied_tx, mut applied_rx) = tokio::sync::mpsc::channel(10);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);

        // Spawn apply loop
        let state_clone = state.clone();
        tokio::spawn(async move {
            apply_loop(state_clone, applied_tx, shutdown_rx, None).await;
        });

        // Wait for applied entries
        let mut applied_count = 0;
        while let Ok(Some((idx, cmd))) = tokio::time::timeout(
            Duration::from_millis(100),
            applied_rx.recv(),
        )
        .await
        {
            applied_count += 1;
            assert_eq!(idx, LogIndex(applied_count));
            assert_eq!(cmd, Bytes::from(format!("cmd{}", applied_count)));

            if applied_count == 3 {
                break;
            }
        }

        assert_eq!(applied_count, 3);

        // Shutdown
        let _ = shutdown_tx.send(());
    }
}
