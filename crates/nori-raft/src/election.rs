//! Leader election logic for Raft.
//!
//! Per Raft §5.2 (Leader Election):
//! - Followers become candidates when election timeout fires
//! - Candidates request votes from all peers
//! - Candidates need majority to become leader
//! - Split votes result in timeout and retry with new term
//! - Randomized timeouts prevent persistent split votes

use crate::config::RaftConfig;
use crate::error::Result;
use crate::state::RaftState;
use crate::transport::RaftTransport;
use crate::types::*;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::time::timeout;

/// Election result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElectionOutcome {
    /// Won election (became leader)
    Won {
        term: Term,
        votes_received: usize,
    },

    /// Lost election (discovered higher term or another leader)
    Lost { current_term: Term },

    /// Election timed out (split vote, retry with new term)
    Timeout,
}

/// Run an election as a candidate.
///
/// Per Raft §5.2:
/// 1. Increment current term
/// 2. Vote for self
/// 3. Reset election timer
/// 4. Send RequestVote RPCs to all peers in parallel
/// 5. Collect votes until:
///    - Receive votes from majority → become leader
///    - Receive AppendEntries from new leader → become follower
///    - Election timeout → start new election
///
/// Returns the election outcome.
pub async fn run_election(
    state: Arc<RaftState>,
    config: &RaftConfig,
    transport: Arc<dyn RaftTransport>,
) -> Result<ElectionOutcome> {
    // Start election (increment term, vote for self)
    let term = state.start_election().await?;

    // Get cluster configuration
    let peers = {
        let volatile = state.volatile_state();
        let cfg = volatile.read().config.clone();
        cfg.all_nodes()
    };

    let node_id = state.node_id().clone();

    // Vote for self
    let mut votes_received = HashSet::new();
    votes_received.insert(node_id.clone());

    // Get last log info for RequestVote
    let last_log_index = state.log_ref().last_index().await;
    let last_log_term = state.log_ref().last_term().await;

    // Send RequestVote to all peers in parallel
    let mut vote_futures = Vec::new();

    for peer in &peers {
        if peer == &node_id {
            continue; // Skip self
        }

        let transport = transport.clone();
        let peer_clone = peer.clone();
        let node_id_clone = node_id.clone();

        let request = RequestVoteRequest {
            term,
            candidate_id: node_id_clone,
            last_log_index,
            last_log_term,
        };

        let fut = async move {
            // Send RequestVote with timeout
            let result = timeout(
                std::time::Duration::from_millis(config.election_timeout_min.as_millis() as u64),
                transport.request_vote(&peer_clone, request),
            )
            .await;

            (peer_clone, result)
        };

        vote_futures.push(fut);
    }

    // Collect votes
    let results = futures::future::join_all(vote_futures).await;

    for (peer, result) in results {
        match result {
            Ok(Ok(response)) => {
                // Check if we're still in the same term
                let current_term = state.current_term();
                if current_term != term {
                    // Term changed (discovered higher term)
                    return Ok(ElectionOutcome::Lost { current_term });
                }

                if response.term > term {
                    // Peer has higher term, step down
                    return Ok(ElectionOutcome::Lost {
                        current_term: response.term,
                    });
                }

                if response.vote_granted {
                    votes_received.insert(peer);
                }
            }
            Ok(Err(_)) => {
                // RPC error (network issue, etc.), ignore
                continue;
            }
            Err(_) => {
                // Timeout, ignore
                continue;
            }
        }
    }

    // Check if we won
    let total_nodes = peers.len();
    let quorum = total_nodes / 2 + 1;

    if votes_received.len() >= quorum {
        // Won election!
        Ok(ElectionOutcome::Won {
            term,
            votes_received: votes_received.len(),
        })
    } else {
        // Didn't get enough votes, election timeout
        Ok(ElectionOutcome::Timeout)
    }
}

/// Election loop (runs continuously in background).
///
/// This task:
/// 1. Waits for election timeout
/// 2. Runs election when timeout fires
/// 3. Transitions to leader if won
/// 4. Repeats if lost or timed out
///
/// Should be spawned as a background task for followers/candidates.
pub async fn election_loop(
    state: Arc<RaftState>,
    config: RaftConfig,
    transport: Arc<dyn RaftTransport>,
    mut timeout_rx: tokio::sync::mpsc::Receiver<()>,
    shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    let mut shutdown_rx = shutdown_rx;

    loop {
        tokio::select! {
            Some(_) = timeout_rx.recv() => {
                // Only run election if we're not leader
                if state.role() == Role::Leader {
                    continue;
                }

                // Run election
                match run_election(state.clone(), &config, transport.clone()).await {
                    Ok(ElectionOutcome::Won { term, votes_received }) => {
                        tracing::info!(
                            term = %term,
                            votes = votes_received,
                            "Won election, becoming leader"
                        );

                        // Transition to leader
                        if let Err(e) = state.become_leader().await {
                            tracing::error!(error = ?e, "Failed to become leader");
                        }
                    }
                    Ok(ElectionOutcome::Lost { current_term }) => {
                        tracing::debug!(
                            term = %current_term,
                            "Lost election (discovered higher term)"
                        );
                        // Already stepped down in state machine
                    }
                    Ok(ElectionOutcome::Timeout) => {
                        tracing::debug!("Election timed out (split vote), will retry");
                        // Will retry on next timeout
                    }
                    Err(e) => {
                        tracing::error!(error = ?e, "Election error");
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("Election loop shutting down");
                break;
            }
        }
    }
}

/// Check if candidate's log is at least as up-to-date as ours.
///
/// Per Raft §5.4.1 (Election restriction):
/// Raft determines which of two logs is more up-to-date by comparing
/// the index and term of the last entries in the logs.
///
/// If the logs have last entries with different terms, then the log
/// with the later term is more up-to-date.
///
/// If the logs end with the same term, then whichever log is longer is
/// more up-to-date.
pub fn is_log_up_to_date(
    candidate_last_term: Term,
    candidate_last_index: LogIndex,
    our_last_term: Term,
    our_last_index: LogIndex,
) -> bool {
    candidate_last_term > our_last_term
        || (candidate_last_term == our_last_term && candidate_last_index >= our_last_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::InMemoryTransport;
    use std::collections::HashMap;
    use tempfile::TempDir;

    async fn create_test_state() -> (Arc<RaftState>, TempDir) {
        use crate::log::RaftLog;

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
    async fn test_is_log_up_to_date_term_comparison() {
        // Candidate with higher term is more up-to-date
        assert!(is_log_up_to_date(
            Term(5),
            LogIndex(10),
            Term(4),
            LogIndex(100)
        ));

        // Our log with higher term is more up-to-date
        assert!(!is_log_up_to_date(
            Term(4),
            LogIndex(100),
            Term(5),
            LogIndex(10)
        ));
    }

    #[tokio::test]
    async fn test_is_log_up_to_date_index_comparison() {
        // Same term, candidate has longer log
        assert!(is_log_up_to_date(
            Term(5),
            LogIndex(100),
            Term(5),
            LogIndex(50)
        ));

        // Same term, our log is longer
        assert!(!is_log_up_to_date(
            Term(5),
            LogIndex(50),
            Term(5),
            LogIndex(100)
        ));

        // Same term and index
        assert!(is_log_up_to_date(
            Term(5),
            LogIndex(50),
            Term(5),
            LogIndex(50)
        ));
    }

    #[tokio::test]
    async fn test_election_outcome_debug() {
        let outcome = ElectionOutcome::Won {
            term: Term(5),
            votes_received: 3,
        };
        assert!(format!("{:?}", outcome).contains("Won"));

        let outcome = ElectionOutcome::Lost {
            current_term: Term(6),
        };
        assert!(format!("{:?}", outcome).contains("Lost"));

        let outcome = ElectionOutcome::Timeout;
        assert!(format!("{:?}", outcome).contains("Timeout"));
    }

    #[tokio::test]
    async fn test_start_election() {
        let (state, _temp) = create_test_state().await;

        // Start election (should increment term)
        let initial_term = state.current_term();
        let new_term = state.start_election().await.unwrap();

        assert_eq!(new_term, initial_term.next());
        assert_eq!(state.current_term(), new_term);
        assert_eq!(state.role(), Role::Candidate);
    }

    // Note: Full election tests with multiple nodes would require
    // setting up in-memory transport with channels and spawning
    // multiple state machines. This is better tested in integration tests.
}
