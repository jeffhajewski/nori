//! gRPC transport + HTTP façade for NoriKV.
//!
//! Implements the gRPC services defined in context/40_protocols.proto:
//! - Kv: Put, Get, Delete
//! - Meta: WatchCluster
//! - Admin: TransferShard, SnapshotShard
//! - Raft: RequestVote, AppendEntries, InstallSnapshot, ReadIndex

pub mod proto {
    //! Generated protobuf types and service traits.
    tonic::include_proto!("norikv.v1");
}

pub mod kv;
pub mod kv_backend;
pub mod meta;
pub mod admin;
pub mod raft;
pub mod raft_service;
pub mod server;
pub mod health;

pub use server::GrpcServer;
pub use health::{HealthService, HealthStatus};
pub use raft::GrpcRaftTransport;
pub use raft_service::RaftService;
pub use kv_backend::{KvBackend, SingleShardBackend};
