//! RPC handler loop for processing incoming Raft RPCs.
//!
//! This module provides the dispatcher that bridges the transport layer
//! (which receives RPC messages) to the RaftState handlers (which process them).
//!
//! # Architecture
//!
//! ```text
//! Transport receives RPC → RpcMessage enum → rpc_handler_loop
//!     ↓
//! Match on message type → Call RaftState::handle_XXX
//!     ↓
//! Send response back via oneshot channel
//! ```

use crate::state::RaftState;
use crate::transport::{RpcMessage, RpcReceiver};
use std::sync::Arc;
use tokio::sync::broadcast;

/// RPC handler loop.
///
/// Continuously receives RPC messages from the transport and dispatches them
/// to the appropriate handler methods in RaftState.
///
/// # Arguments
/// - `state`: Shared Raft state
/// - `rpc_rx`: Channel receiving incoming RPC messages from transport
/// - `shutdown_rx`: Shutdown signal to stop the loop
///
/// # Shutdown
/// The loop exits when either:
/// - Shutdown signal is received
/// - RPC receiver channel is closed (peer disconnected)
pub async fn rpc_handler_loop(
    state: Arc<RaftState>,
    mut rpc_rx: RpcReceiver,
    election_timer: Arc<crate::timer::ElectionTimer>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    loop {
        tokio::select! {
            // Receive next RPC message
            msg_opt = rpc_rx.recv() => {
                match msg_opt {
                    Some(msg) => {
                        // Dispatch to handler
                        handle_rpc_message(state.clone(), election_timer.clone(), msg).await;
                    }
                    None => {
                        // Channel closed, peer disconnected
                        tracing::debug!("RPC channel closed, exiting handler loop");
                        break;
                    }
                }
            }
            // Shutdown signal
            _ = shutdown_rx.recv() => {
                tracing::info!("RPC handler loop shutting down");
                break;
            }
        }
    }
}

/// Dispatch a single RPC message to the appropriate handler.
///
/// Matches on message type and calls the corresponding RaftState handler.
/// Sends the response back via the oneshot channel.
async fn handle_rpc_message(
    state: Arc<RaftState>,
    election_timer: Arc<crate::timer::ElectionTimer>,
    msg: RpcMessage,
) {
    match msg {
        RpcMessage::RequestVote { request, response_tx } => {
            let response = state.handle_request_vote(request).await;
            match response {
                Ok(resp) => {
                    let _ = response_tx.send(resp);
                }
                Err(e) => {
                    tracing::error!(error = ?e, "Failed to handle RequestVote");
                    // Can't send error back via oneshot (only one value allowed)
                    // Transport will timeout waiting for response
                }
            }
        }

        RpcMessage::AppendEntries { request, response_tx } => {
            let response = state.handle_append_entries(request).await;
            match response {
                Ok(resp) => {
                    // Reset election timer if we accepted the AppendEntries
                    // (i.e., valid heartbeat from current leader)
                    if resp.success {
                        election_timer.reset();
                    }
                    let _ = response_tx.send(resp);
                }
                Err(e) => {
                    tracing::error!(error = ?e, "Failed to handle AppendEntries");
                }
            }
        }

        RpcMessage::InstallSnapshot { request, response_tx } => {
            let response = state.handle_install_snapshot(request).await;
            match response {
                Ok(resp) => {
                    let _ = response_tx.send(resp);
                }
                Err(e) => {
                    tracing::error!(error = ?e, "Failed to handle InstallSnapshot");
                }
            }
        }

        RpcMessage::ReadIndex { request, response_tx } => {
            let response = state.handle_read_index(request).await;
            match response {
                Ok(resp) => {
                    let _ = response_tx.send(resp);
                }
                Err(e) => {
                    tracing::error!(error = ?e, "Failed to handle ReadIndex");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RaftConfig;
    use crate::log::RaftLog;
    use crate::transport::{InMemoryTransport, RaftTransport};
    use crate::types::*;
    use std::collections::HashMap;
    use tempfile::TempDir;
    use tokio::sync::broadcast;

    async fn create_test_state() -> (Arc<RaftState>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let (log, _) = RaftLog::open(temp_dir.path()).await.unwrap();

        let config = RaftConfig::default();
        let transport: Arc<dyn RaftTransport> = Arc::new(InMemoryTransport::new(
            NodeId::new("n1"),
            HashMap::new(),
        ));

        let initial_config = ConfigEntry::Single(vec![NodeId::new("n1")]);

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
    async fn test_rpc_handler_request_vote() {
        let (state, _temp) = create_test_state().await;

        // Create channel for RPC messages
        let (rpc_tx, rpc_rx) = tokio::sync::mpsc::channel(10);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        // Create election timer
        let config = RaftConfig::default();
        let election_timer = Arc::new(crate::timer::ElectionTimer::new(config));

        // Spawn handler loop
        let state_clone = state.clone();
        let handler_task = tokio::spawn(async move {
            rpc_handler_loop(state_clone, rpc_rx, election_timer, shutdown_rx).await;
        });

        // Send RequestVote RPC
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let request = RequestVoteRequest {
            term: Term(5),
            candidate_id: NodeId::new("n2"),
            last_log_index: LogIndex(0),
            last_log_term: Term(0),
        };

        rpc_tx
            .send(RpcMessage::RequestVote {
                request,
                response_tx,
            })
            .await
            .unwrap();

        // Wait for response
        let response = response_rx.await.unwrap();
        assert_eq!(response.term, Term(5));
        assert!(response.vote_granted); // Should grant vote (log is up-to-date)

        // Shutdown
        let _ = shutdown_tx.send(());
        handler_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_rpc_handler_append_entries() {
        let (state, _temp) = create_test_state().await;

        // Set term to 1
        state.set_current_term(Term(1));

        // Create channel for RPC messages
        let (rpc_tx, rpc_rx) = tokio::sync::mpsc::channel(10);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        // Create election timer
        let config = RaftConfig::default();
        let election_timer = Arc::new(crate::timer::ElectionTimer::new(config));

        // Spawn handler loop
        let state_clone = state.clone();
        let handler_task = tokio::spawn(async move {
            rpc_handler_loop(state_clone, rpc_rx, election_timer, shutdown_rx).await;
        });

        // Send AppendEntries RPC (heartbeat)
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let request = AppendEntriesRequest {
            term: Term(1),
            leader_id: NodeId::new("n2"),
            prev_log_index: LogIndex::ZERO,
            prev_log_term: Term::ZERO,
            entries: vec![],
            leader_commit: LogIndex::ZERO,
        };

        rpc_tx
            .send(RpcMessage::AppendEntries {
                request,
                response_tx,
            })
            .await
            .unwrap();

        // Wait for response
        let response = response_rx.await.unwrap();
        assert_eq!(response.term, Term(1));
        assert!(response.success); // Heartbeat should succeed

        // Shutdown
        let _ = shutdown_tx.send(());
        handler_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_rpc_handler_shutdown() {
        let (state, _temp) = create_test_state().await;

        // Create channel for RPC messages
        let (_rpc_tx, rpc_rx) = tokio::sync::mpsc::channel(10);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        // Create election timer
        let config = RaftConfig::default();
        let election_timer = Arc::new(crate::timer::ElectionTimer::new(config));

        // Spawn handler loop
        let handler_task = tokio::spawn(async move {
            rpc_handler_loop(state, rpc_rx, election_timer, shutdown_rx).await;
        });

        // Send shutdown immediately
        let _ = shutdown_tx.send(());

        // Handler should exit cleanly
        handler_task.await.unwrap();
    }
}
