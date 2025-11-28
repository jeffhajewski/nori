# Building a Message Queue

Simple message queue with consumer position tracking using nori-wal.

## Table of contents

---

## Problem

You want to build a message queue where:
- Messages are durably stored
- Multiple consumers can read independently
- Each consumer tracks its own position
- Messages can be replayed
- At-least-once delivery semantics

## Solution

```rust
use nori_wal::{Wal, WalConfig, Record, Position};
use serde::{Serialize, Deserialize};
use bytes::Bytes;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

/// A message in the queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: u64,
    pub topic: String,
    pub payload: Bytes,
    pub timestamp: u64,
}

/// Consumer position tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerOffset {
    pub consumer_id: String,
    pub segment_id: u64,
    pub offset: u64,
    pub last_message_id: u64,
}

/// Message queue implementation
pub struct MessageQueue {
    wal: Wal,
    offsets: HashMap<String, Position>,
    next_message_id: u64,
}

impl MessageQueue {
    /// Opens or creates a message queue
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();

        let config = WalConfig {
            dir: path.join("messages"),
            max_segment_size: 128 * 1024 * 1024,
            fsync_policy: nori_wal::FsyncPolicy::Batch(
                std::time::Duration::from_millis(5)
            ),
            preallocate: true,
            node_id: 0,
        };

        let (wal, recovery_info) = Wal::open(config).await?;

        println!("Message queue recovered:");
        println!("  Messages: {}", recovery_info.valid_records);
        println!("  Segments: {}", recovery_info.segments_scanned);

        // Find highest message ID
        let mut next_message_id = 0u64;
        let mut reader = wal.read_from(Position { segment_id: 0, offset: 0 }).await?;

        while let Some((record, _)) = reader.next_record().await? {
            if let Ok(msg) = serde_json::from_slice::<Message>(&record.value) {
                next_message_id = next_message_id.max(msg.id + 1);
            }
        }

        Ok(Self {
            wal,
            offsets: HashMap::new(),
            next_message_id,
        })
    }

    /// Publishes a message to the queue
    pub async fn publish(&mut self, topic: &str, payload: Bytes) -> Result<u64> {
        let message = Message {
            id: self.next_message_id,
            topic: topic.to_string(),
            payload,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
        };

        // Serialize message
        let message_bytes = serde_json::to_vec(&message)?;

        // Write to WAL
        let record = Record::put(&message.id.to_le_bytes(), &message_bytes);
        self.wal.append(&record).await?;

        let message_id = self.next_message_id;
        self.next_message_id += 1;

        Ok(message_id)
    }

    /// Syncs messages to disk
    pub async fn sync(&self) -> Result<()> {
        self.wal.sync().await?;
        Ok(())
    }

    /// Creates a consumer at the beginning of the queue
    pub fn consumer(&mut self, consumer_id: &str) -> Consumer {
        let position = self.offsets.get(consumer_id).copied()
            .unwrap_or(Position { segment_id: 0, offset: 0 });

        Consumer {
            id: consumer_id.to_string(),
            position,
        }
    }

    /// Consumes the next message for a consumer
    pub async fn consume(&mut self, consumer: &mut Consumer) -> Result<Option<Message>> {
        let mut reader = self.wal.read_from(consumer.position).await?;

        if let Some((record, position)) = reader.next_record().await? {
            if let Ok(message) = serde_json::from_slice::<Message>(&record.value) {
                // Update consumer position
                consumer.position = position;
                self.offsets.insert(consumer.id.clone(), position);

                return Ok(Some(message));
            }
        }

        Ok(None)
    }

    /// Commits a consumer's current position
    pub async fn commit(&mut self, consumer: &Consumer) -> Result<()> {
        self.offsets.insert(consumer.id.clone(), consumer.position);
        Ok(())
    }

    /// Resets a consumer to the beginning
    pub fn reset_consumer(&mut self, consumer_id: &str) {
        self.offsets.insert(
            consumer_id.to_string(),
            Position { segment_id: 0, offset: 0 },
        );
    }

    /// Seeks a consumer to a specific message ID
    pub async fn seek(&mut self, consumer: &mut Consumer, message_id: u64) -> Result<()> {
        let mut reader = self.wal.read_from(Position { segment_id: 0, offset: 0 }).await?;

        while let Some((record, position)) = reader.next_record().await? {
            if let Ok(message) = serde_json::from_slice::<Message>(&record.value) {
                if message.id == message_id {
                    consumer.position = position;
                    self.offsets.insert(consumer.id.clone(), position);
                    return Ok(());
                }
            }
        }

        Err(anyhow::anyhow!("Message ID {} not found", message_id))
    }

    /// Returns the current lag for a consumer
    pub async fn lag(&self, consumer: &Consumer) -> Result<u64> {
        let mut count = 0u64;
        let mut reader = self.wal.read_from(consumer.position).await?;

        while let Some(_) = reader.next_record().await? {
            count += 1;
        }

        Ok(count)
    }

    /// Gracefully closes the queue
    pub async fn close(self) -> Result<()> {
        self.wal.close().await?;
        Ok(())
    }
}

/// A consumer that reads from the queue
#[derive(Debug, Clone)]
pub struct Consumer {
    pub id: String,
    position: Position,
}

// Example usage
#[tokio::main]
async fn main() -> Result<()> {
    let mut queue = MessageQueue::open("./message_queue").await?;

    // Publish messages
    for i in 0..10 {
        let payload = format!("Message {}", i);
        queue.publish("orders", Bytes::from(payload)).await?;
    }

    queue.sync().await?;

    // Create consumers
    let mut consumer_a = queue.consumer("consumer-a");
    let mut consumer_b = queue.consumer("consumer-b");

    // Consumer A reads 5 messages
    for _ in 0..5 {
        if let Some(msg) = queue.consume(&mut consumer_a).await? {
            println!("Consumer A: {:?}", msg);
        }
    }
    queue.commit(&consumer_a).await?;

    // Consumer B reads all messages
    while let Some(msg) = queue.consume(&mut consumer_b).await? {
        println!("Consumer B: {:?}", msg);
    }
    queue.commit(&consumer_b).await?;

    // Check lag
    let lag = queue.lag(&consumer_a).await?;
    println!("Consumer A lag: {} messages", lag);

    queue.close().await?;

    Ok(())
}
```

## How It Works

### 1. Message Publishing

Messages are appended to the WAL:

```rust
pub async fn publish(&mut self, topic: &str, payload: Bytes) -> Result<u64> {
    let message = Message {
        id: self.next_message_id,
        topic: topic.to_string(),
        payload,
        timestamp: current_timestamp(),
    };

    let message_bytes = serde_json::to_vec(&message)?;
    let record = Record::put(&message.id.to_le_bytes(), &message_bytes);
    self.wal.append(&record).await?;

    self.next_message_id += 1;
    Ok(message.id)
}
```

### 2. Consumer Position Tracking

Each consumer maintains its own position:

```rust
pub struct Consumer {
    pub id: String,
    position: Position,  // Current position in WAL
}

pub async fn consume(&mut self, consumer: &mut Consumer) -> Result<Option<Message>> {
    let mut reader = self.wal.read_from(consumer.position).await?;

    if let Some((record, new_position)) = reader.next_record().await? {
        consumer.position = new_position;  // Update position
        let message = serde_json::from_slice(&record.value)?;
        Ok(Some(message))
    } else {
        Ok(None)  // No more messages
    }
}
```

### 3. Offset Management

Consumer offsets are tracked in memory and persisted on commit:

```rust
pub async fn commit(&mut self, consumer: &Consumer) -> Result<()> {
    // Store offset in HashMap
    self.offsets.insert(consumer.id.clone(), consumer.position);

    // For durability, you could also write offsets to a separate WAL
    Ok(())
}
```

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_publish_and_consume() {
        let dir = TempDir::new().unwrap();
        let mut queue = MessageQueue::open(dir.path()).await.unwrap();

        // Publish messages
        queue.publish("test", Bytes::from("msg1")).await.unwrap();
        queue.publish("test", Bytes::from("msg2")).await.unwrap();

        // Consume
        let mut consumer = queue.consumer("test-consumer");
        let msg1 = queue.consume(&mut consumer).await.unwrap().unwrap();
        let msg2 = queue.consume(&mut consumer).await.unwrap().unwrap();

        assert_eq!(msg1.id, 0);
        assert_eq!(msg2.id, 1);
    }

    #[tokio::test]
    async fn test_multiple_consumers() {
        let dir = TempDir::new().unwrap();
        let mut queue = MessageQueue::open(dir.path()).await.unwrap();

        // Publish 10 messages
        for i in 0..10 {
            queue.publish("test", Bytes::from(format!("msg{}", i))).await.unwrap();
        }

        // Consumer A reads 5
        let mut consumer_a = queue.consumer("a");
        for _ in 0..5 {
            queue.consume(&mut consumer_a).await.unwrap();
        }
        queue.commit(&consumer_a).await.unwrap();

        // Consumer B reads all 10
        let mut consumer_b = queue.consumer("b");
        let mut count = 0;
        while let Some(_) = queue.consume(&mut consumer_b).await.unwrap() {
            count += 1;
        }
        assert_eq!(count, 10);

        // Consumer A still has 5 messages left
        let lag = queue.lag(&consumer_a).await.unwrap();
        assert_eq!(lag, 5);
    }

    #[tokio::test]
    async fn test_consumer_recovery() {
        let dir = TempDir::new().unwrap();

        // Write messages and consume some
        {
            let mut queue = MessageQueue::open(dir.path()).await.unwrap();
            for i in 0..10 {
                queue.publish("test", Bytes::from(format!("msg{}", i))).await.unwrap();
            }

            let mut consumer = queue.consumer("persistent");
            for _ in 0..3 {
                queue.consume(&mut consumer).await.unwrap();
            }
            queue.commit(&consumer).await.unwrap();
            queue.sync().await.unwrap();
        }

        // Reopen and verify consumer position
        {
            let mut queue = MessageQueue::open(dir.path()).await.unwrap();
            let mut consumer = queue.consumer("persistent");

            // Should start from message 3
            let msg = queue.consume(&mut consumer).await.unwrap().unwrap();
            assert_eq!(msg.id, 3);
        }
    }

    #[tokio::test]
    async fn test_seek() {
        let dir = TempDir::new().unwrap();
        let mut queue = MessageQueue::open(dir.path()).await.unwrap();

        // Publish 10 messages
        for i in 0..10 {
            queue.publish("test", Bytes::from(format!("msg{}", i))).await.unwrap();
        }

        let mut consumer = queue.consumer("seeker");

        // Seek to message ID 5
        queue.seek(&mut consumer, 5).await.unwrap();

        // Next message should be ID 5
        let msg = queue.consume(&mut consumer).await.unwrap().unwrap();
        assert_eq!(msg.id, 5);
    }
}
```

## Production Considerations

### 1. Persistent Offset Storage

Store consumer offsets durably:

```rust
pub struct MessageQueue {
    wal: Wal,
    offset_wal: Wal,  // Separate WAL for offsets
    offsets: HashMap<String, Position>,
    next_message_id: u64,
}

impl MessageQueue {
    pub async fn commit(&mut self, consumer: &Consumer) -> Result<()> {
        // Update in-memory
        self.offsets.insert(consumer.id.clone(), consumer.position);

        // Persist to offset WAL
        let offset = ConsumerOffset {
            consumer_id: consumer.id.clone(),
            segment_id: consumer.position.segment_id,
            offset: consumer.position.offset,
            last_message_id: 0, // Track if needed
        };

        let offset_bytes = serde_json::to_vec(&offset)?;
        let record = Record::put(consumer.id.as_bytes(), &offset_bytes);
        self.offset_wal.append(&record).await?;

        Ok(())
    }

    pub async fn load_offsets(&mut self) -> Result<()> {
        let mut reader = self.offset_wal.read_from(Position { segment_id: 0, offset: 0 }).await?;

        while let Some((record, _)) = reader.next_record().await? {
            if let Ok(offset) = serde_json::from_slice::<ConsumerOffset>(&record.value) {
                self.offsets.insert(
                    offset.consumer_id,
                    Position {
                        segment_id: offset.segment_id,
                        offset: offset.offset,
                    },
                );
            }
        }

        Ok(())
    }
}
```

### 2. Message Retention

Delete old segments:

```rust
impl MessageQueue {
    /// Deletes messages older than retention period
    pub async fn cleanup(&mut self, retention_seconds: u64) -> Result<u64> {
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
            - retention_seconds;

        let mut oldest_position = Position { segment_id: u64::MAX, offset: 0 };

        // Find oldest position still in use by any consumer
        for position in self.offsets.values() {
            if position.segment_id < oldest_position.segment_id {
                oldest_position = *position;
            }
        }

        // Delete segments before oldest consumer position
        if oldest_position.segment_id > 0 {
            self.wal.delete_segments_before(oldest_position.segment_id).await?;
        }

        Ok(oldest_position.segment_id)
    }
}
```

### 3. Topic Filtering

Filter messages by topic:

```rust
pub async fn consume_topic(
    &mut self,
    consumer: &mut Consumer,
    topic: &str,
) -> Result<Option<Message>> {
    loop {
        let mut reader = self.wal.read_from(consumer.position).await?;

        if let Some((record, position)) = reader.next_record().await? {
            consumer.position = position;

            if let Ok(message) = serde_json::from_slice::<Message>(&record.value) {
                if message.topic == topic {
                    return Ok(Some(message));
                }
                // Skip messages from other topics
                continue;
            }
        } else {
            return Ok(None);
        }
    }
}
```

### 4. Batch Consumption

Consume multiple messages at once:

```rust
pub async fn consume_batch(
    &mut self,
    consumer: &mut Consumer,
    max_messages: usize,
) -> Result<Vec<Message>> {
    let mut messages = Vec::new();
    let mut reader = self.wal.read_from(consumer.position).await?;

    for _ in 0..max_messages {
        if let Some((record, position)) = reader.next_record().await? {
            if let Ok(message) = serde_json::from_slice::<Message>(&record.value) {
                messages.push(message);
                consumer.position = position;
            }
        } else {
            break;
        }
    }

    Ok(messages)
}
```

### 5. Monitoring

Track queue metrics:

```rust
// Messages published
metrics.counter("queue.messages.published", 1, &[("topic", topic)]);

// Consumer lag
metrics.gauge("queue.consumer.lag", lag, &[("consumer", consumer.id)]);

// Consumption rate
metrics.counter("queue.messages.consumed", 1, &[("consumer", consumer.id)]);
```

## Conclusion

This recipe demonstrates:
- Using WAL as a message log
- Independent consumer position tracking
- At-least-once delivery semantics
- Message replay and seeking

For distributed message queues, combine with [Replication recipe](replication.md).
