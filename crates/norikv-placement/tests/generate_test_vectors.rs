//! Generate test vectors for cross-language hash validation.
//!
//! This test generates a JSON file containing hash test vectors that can be used
//! by all client SDKs to verify that their hash implementations match the server.

use norikv_placement::{get_shard_for_key, jump_consistent_hash, xxhash64};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct TestVector {
    key: String,
    key_hex: String,
    #[serde(serialize_with = "serialize_u64_as_string")]
    xxhash64: u64,
    shard_1024: u32,
    shard_256: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct JumpHashVector {
    #[serde(serialize_with = "serialize_u64_as_string")]
    hash: u64,
    buckets_100: u32,
    buckets_1024: u32,
}

fn serialize_u64_as_string<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&value.to_string())
}

#[derive(Debug, Serialize, Deserialize)]
struct TestVectors {
    description: String,
    generated_at: String,
    key_hash_vectors: Vec<TestVector>,
    jump_hash_vectors: Vec<JumpHashVector>,
}

#[test]
fn generate_test_vectors() {
    let long_key = "x".repeat(1000);
    let test_keys = vec![
        "hello",
        "world",
        "user:123",
        "user:456",
        "order:789",
        "session:abc",
        "config:feature-flag",
        "metrics:cpu",
        "cache:user:profile:12345",
        "db:table:users:index:email",
        // UTF-8 test cases
        "Hello 世界",
        "🌍🚀",
        // Edge cases
        "",
        "a",
        long_key.as_str(),
    ];

    let mut key_vectors = Vec::new();
    for key in test_keys {
        let key_bytes = key.as_bytes();
        let hash = xxhash64(key_bytes);
        let shard_1024 = get_shard_for_key(key_bytes, 1024);
        let shard_256 = get_shard_for_key(key_bytes, 256);

        key_vectors.push(TestVector {
            key: key.to_string(),
            key_hex: hex::encode(key_bytes),
            xxhash64: hash,
            shard_1024,
            shard_256,
        });
    }

    // Jump hash test vectors with specific hash values
    let test_hashes = vec![
        0u64,
        1,
        12345678901234567890,
        0xFFFFFFFFFFFFFFFF, // max u64
        0x8000000000000000, // high bit set
        1234567890,
        9876543210,
    ];

    let mut jump_vectors = Vec::new();
    for hash in test_hashes {
        jump_vectors.push(JumpHashVector {
            hash,
            buckets_100: jump_consistent_hash(hash, 100),
            buckets_1024: jump_consistent_hash(hash, 1024),
        });
    }

    let vectors = TestVectors {
        description: "Cross-language hash validation test vectors for NoriKV".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        key_hash_vectors: key_vectors,
        jump_hash_vectors: jump_vectors,
    };

    let json = serde_json::to_string_pretty(&vectors).unwrap();

    // Write to a file that can be committed to the repo
    let output_path = std::env::var("CARGO_MANIFEST_DIR")
        .map(|dir| format!("{}/../../test-vectors/hash-vectors.json", dir))
        .unwrap_or_else(|_| "test-vectors/hash-vectors.json".to_string());

    std::fs::create_dir_all(std::path::Path::new(&output_path).parent().unwrap()).ok();
    std::fs::write(&output_path, json).expect("Failed to write test vectors");

    println!("Generated test vectors at: {}", output_path);
    println!("Total key vectors: {}", vectors.key_hash_vectors.len());
    println!("Total jump hash vectors: {}", vectors.jump_hash_vectors.len());
}
