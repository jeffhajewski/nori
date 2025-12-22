//! Vector gRPC integration tests for norikv-server.
//!
//! These tests verify the Vector gRPC service works correctly.
//! Note: Uses MockVectorBackend since full storage integration is pending.

use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Duration as TokioDuration;

// Re-export the proto types and client stubs
use norikv_transport_grpc::proto::{
    vector_client::VectorClient,
    CreateVectorIndexRequest, DropVectorIndexRequest,
    VectorInsertRequest, VectorDeleteRequest,
    VectorSearchRequest, VectorGetRequest,
    DistanceFunction, VectorIndexType,
};
use norikv_transport_grpc::{GrpcServer, KvBackend, VectorBackend};
use nori_raft::{LogIndex, NodeId, RaftError, Term};

/// Mock vector backend for testing.
struct MockVectorBackend;

#[async_trait::async_trait]
impl VectorBackend for MockVectorBackend {
    async fn create_index(
        &self,
        _namespace: String,
        _dimensions: usize,
        _distance: norikv_transport_grpc::DistanceFunction,
        _index_type: norikv_transport_grpc::VectorIndexType,
    ) -> Result<bool, RaftError> {
        Ok(true)
    }

    async fn drop_index(&self, _namespace: String) -> Result<bool, RaftError> {
        Ok(true)
    }

    async fn insert(
        &self,
        _namespace: String,
        _id: String,
        _vector: Vec<f32>,
    ) -> Result<LogIndex, RaftError> {
        Ok(LogIndex(42))
    }

    async fn delete(&self, _namespace: String, _id: String) -> Result<bool, RaftError> {
        Ok(true)
    }

    async fn search(
        &self,
        _namespace: &str,
        _query: &[f32],
        k: usize,
        include_vectors: bool,
    ) -> Result<Vec<norikv_transport_grpc::VectorMatch>, RaftError> {
        let matches: Vec<norikv_transport_grpc::VectorMatch> = (0..k.min(3))
            .map(|i| norikv_transport_grpc::VectorMatch {
                id: format!("vec-{}", i),
                distance: i as f32 * 0.1,
                vector: if include_vectors {
                    Some(vec![0.1, 0.2, 0.3, 0.4])
                } else {
                    None
                },
            })
            .collect();
        Ok(matches)
    }

    async fn get(&self, _namespace: &str, _id: &str) -> Result<Option<Vec<f32>>, RaftError> {
        Ok(Some(vec![0.1, 0.2, 0.3, 0.4]))
    }

    fn is_leader(&self) -> bool {
        true
    }

    fn leader(&self) -> Option<NodeId> {
        Some(NodeId::new("test-leader"))
    }

    fn current_term(&self) -> Term {
        Term(1)
    }

    fn commit_index(&self) -> LogIndex {
        LogIndex(100)
    }
}

/// Mock KV backend (required for GrpcServer).
struct MockKvBackend;

#[async_trait::async_trait]
impl KvBackend for MockKvBackend {
    async fn get(&self, _key: &[u8]) -> Result<Option<(Bytes, Term, LogIndex)>, RaftError> {
        Ok(None)
    }

    async fn put(&self, _key: Bytes, _value: Bytes, _ttl: Option<Duration>) -> Result<LogIndex, RaftError> {
        Ok(LogIndex(1))
    }

    async fn delete(&self, _key: Bytes) -> Result<LogIndex, RaftError> {
        Ok(LogIndex(1))
    }

    fn is_leader(&self) -> bool {
        true
    }

    fn leader(&self) -> Option<NodeId> {
        Some(NodeId::new("test-leader"))
    }

    fn current_term(&self) -> Term {
        Term(1)
    }

    fn commit_index(&self) -> LogIndex {
        LogIndex(100)
    }
}

async fn start_test_server(port: u16) -> GrpcServer {
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
    let kv_backend: Arc<dyn KvBackend> = Arc::new(MockKvBackend);
    let vector_backend: Arc<dyn VectorBackend> = Arc::new(MockVectorBackend);

    let mut server = GrpcServer::with_backend(addr, kv_backend)
        .with_vector_backend(vector_backend);

    server.start().await.expect("Failed to start gRPC server");
    server
}

#[tokio::test(flavor = "multi_thread")]
async fn test_vector_create_and_drop_index() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    tracing::info!("Starting vector create/drop index integration test");

    // Start gRPC server on a unique port
    let server = start_test_server(50060).await;
    tokio::time::sleep(TokioDuration::from_millis(100)).await;

    // Connect to the gRPC server
    let mut client = VectorClient::connect("http://127.0.0.1:50060")
        .await
        .expect("Failed to connect to gRPC server");

    tracing::info!("Vector gRPC client connected");

    // Test 1: Create index
    let create_req = CreateVectorIndexRequest {
        namespace: "test_index".to_string(),
        dimensions: 128,
        distance: DistanceFunction::Cosine as i32,
        index_type: VectorIndexType::Hnsw as i32,
        idempotency_key: "create-idx-1".to_string(),
    };

    let create_resp = client.create_index(create_req)
        .await
        .expect("Create index request failed")
        .into_inner();

    assert!(create_resp.created, "Create index should succeed");
    tracing::info!("Create index succeeded");

    // Test 2: Drop index
    let drop_req = DropVectorIndexRequest {
        namespace: "test_index".to_string(),
        idempotency_key: "drop-idx-1".to_string(),
    };

    let drop_resp = client.drop_index(drop_req)
        .await
        .expect("Drop index request failed")
        .into_inner();

    assert!(drop_resp.dropped, "Drop index should succeed");
    tracing::info!("Drop index succeeded");

    // Shutdown
    server.shutdown().await.expect("Failed to shutdown server");
    tracing::info!("Vector create/drop index integration test completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_vector_insert_and_search() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    tracing::info!("Starting vector insert/search integration test");

    // Start gRPC server on a unique port
    let server = start_test_server(50061).await;
    tokio::time::sleep(TokioDuration::from_millis(100)).await;

    // Connect to the gRPC server
    let mut client = VectorClient::connect("http://127.0.0.1:50061")
        .await
        .expect("Failed to connect to gRPC server");

    tracing::info!("Vector gRPC client connected");

    // Test 1: Insert vector
    let insert_req = VectorInsertRequest {
        namespace: "embeddings".to_string(),
        id: "doc-1".to_string(),
        vector: vec![0.1, 0.2, 0.3, 0.4],
        idempotency_key: "insert-1".to_string(),
    };

    let insert_resp = client.insert(insert_req)
        .await
        .expect("Insert request failed")
        .into_inner();

    assert!(insert_resp.inserted, "Insert should succeed");
    assert!(insert_resp.version.is_some(), "Insert should return version");
    tracing::info!("Insert succeeded with version: {:?}", insert_resp.version);

    // Test 2: Search for similar vectors
    let search_req = VectorSearchRequest {
        namespace: "embeddings".to_string(),
        query: vec![0.1, 0.2, 0.3, 0.4],
        k: 5,
        include_vectors: true,
        consistency: String::new(),
    };

    let search_resp = client.search(search_req)
        .await
        .expect("Search request failed")
        .into_inner();

    assert!(!search_resp.matches.is_empty(), "Search should return matches");
    assert!(search_resp.matches.len() <= 5, "Should return at most k matches");

    // Verify first match has vector data (since include_vectors=true)
    let first_match = &search_resp.matches[0];
    assert!(!first_match.id.is_empty(), "Match should have ID");
    assert!(!first_match.vector.is_empty(), "Match should include vector");

    tracing::info!("Search succeeded with {} matches", search_resp.matches.len());

    // Test 3: Get specific vector
    let get_req = VectorGetRequest {
        namespace: "embeddings".to_string(),
        id: "doc-1".to_string(),
        consistency: String::new(),
    };

    let get_resp = client.get(get_req)
        .await
        .expect("Get request failed")
        .into_inner();

    assert!(get_resp.found, "Vector should be found");
    assert!(!get_resp.vector.is_empty(), "Should return vector data");
    tracing::info!("Get succeeded: vector has {} dimensions", get_resp.vector.len());

    // Shutdown
    server.shutdown().await.expect("Failed to shutdown server");
    tracing::info!("Vector insert/search integration test completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_vector_delete() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    tracing::info!("Starting vector delete integration test");

    // Start gRPC server on a unique port
    let server = start_test_server(50062).await;
    tokio::time::sleep(TokioDuration::from_millis(100)).await;

    // Connect to the gRPC server
    let mut client = VectorClient::connect("http://127.0.0.1:50062")
        .await
        .expect("Failed to connect to gRPC server");

    tracing::info!("Vector gRPC client connected");

    // Delete vector
    let delete_req = VectorDeleteRequest {
        namespace: "embeddings".to_string(),
        id: "doc-to-delete".to_string(),
        idempotency_key: "delete-1".to_string(),
    };

    let delete_resp = client.delete(delete_req)
        .await
        .expect("Delete request failed")
        .into_inner();

    assert!(delete_resp.deleted, "Delete should succeed");
    tracing::info!("Delete succeeded");

    // Shutdown
    server.shutdown().await.expect("Failed to shutdown server");
    tracing::info!("Vector delete integration test completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_vector_validation_errors() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    tracing::info!("Starting vector validation errors integration test");

    // Start gRPC server on a unique port
    let server = start_test_server(50063).await;
    tokio::time::sleep(TokioDuration::from_millis(100)).await;

    // Connect to the gRPC server
    let mut client = VectorClient::connect("http://127.0.0.1:50063")
        .await
        .expect("Failed to connect to gRPC server");

    // Test 1: Empty namespace should fail
    let bad_req = CreateVectorIndexRequest {
        namespace: "".to_string(), // Empty namespace
        dimensions: 128,
        distance: DistanceFunction::Cosine as i32,
        index_type: VectorIndexType::Hnsw as i32,
        idempotency_key: "".to_string(),
    };

    let result = client.create_index(bad_req).await;
    assert!(result.is_err(), "Empty namespace should fail validation");
    tracing::info!("Empty namespace correctly rejected");

    // Test 2: Zero dimensions should fail
    let bad_req2 = CreateVectorIndexRequest {
        namespace: "test".to_string(),
        dimensions: 0, // Zero dimensions
        distance: DistanceFunction::Cosine as i32,
        index_type: VectorIndexType::Hnsw as i32,
        idempotency_key: "".to_string(),
    };

    let result2 = client.create_index(bad_req2).await;
    assert!(result2.is_err(), "Zero dimensions should fail validation");
    tracing::info!("Zero dimensions correctly rejected");

    // Test 3: Empty vector in insert should fail
    let bad_insert = VectorInsertRequest {
        namespace: "test".to_string(),
        id: "doc-1".to_string(),
        vector: vec![], // Empty vector
        idempotency_key: "".to_string(),
    };

    let result3 = client.insert(bad_insert).await;
    assert!(result3.is_err(), "Empty vector should fail validation");
    tracing::info!("Empty vector correctly rejected");

    // Shutdown
    server.shutdown().await.expect("Failed to shutdown server");
    tracing::info!("Vector validation errors integration test completed");
}
