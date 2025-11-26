//! gRPC transport + HTTP façade for NoriKV.
//!
//! Implements the gRPC services defined in context/40_protocols.proto:
//! - Kv: Put, Get, Delete
//! - Meta: WatchCluster
//! - Admin: TransferShard, SnapshotShard
//! - Raft: RequestVote, AppendEntries, InstallSnapshot, ReadIndex
//! - Vector: CreateIndex, DropIndex, Insert, Delete, Search, Get

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
pub mod vector;
pub mod vector_backend;

pub use server::GrpcServer;
pub use health::{HealthService, HealthStatus};
pub use raft::GrpcRaftTransport;
pub use raft_service::RaftService;
pub use kv_backend::{KvBackend, SingleShardBackend};
pub use meta::{MetaService, ClusterViewProvider, ClusterView, ClusterNode, ShardInfo, ShardReplica};
pub use admin::{AdminService, ShardManagerOps, ShardMetadata};
pub use vector::VectorService;
pub use vector_backend::{VectorBackend, SingleShardVectorBackend, DistanceFunction, VectorIndexType, VectorMatch};
