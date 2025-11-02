//! gRPC transport + HTTP façade for NoriKV.
//!
//! Implements the gRPC services defined in context/40_protocols.proto:
//! - Kv: Put, Get, Delete
//! - Meta: WatchCluster
//! - Admin: TransferShard, SnapshotShard

pub mod proto {
    //! Generated protobuf types and service traits.
    tonic::include_proto!("norikv.v1");
}

pub mod kv;
pub mod meta;
pub mod admin;
pub mod server;

pub use server::GrpcServer;
