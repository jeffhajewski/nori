# Event Sourcing with nori-wal

Building an event-sourced system using WAL for durable event storage.

## Table of contents

---

## Problem

You want to build an event-sourced application where:
- All state changes are captured as events
- Events are immutable and append-only
- State can be reconstructed by replaying events
- Events survive crashes and restarts

## Solution

```rust
use nori_wal::{Wal, WalConfig, Record, Position};
use serde::{Serialize, Deserialize};
use bytes::Bytes;
use anyhow::Result;
use std::path::PathBuf;

/// Base event trait with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_id: u64,
    pub event_type: String,
    pub timestamp: u64,
    pub payload: Bytes,
}

/// Example: Bank account events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccountEvent {
    AccountOpened { account_id: String, initial_balance: i64 },
    MoneyDeposited { account_id: String, amount: i64 },
    MoneyWithdrawn { account_id: String, amount: i64 },
    AccountClosed { account_id: String },
}

/// Account state (derived from events)
#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub balance: i64,
    pub is_closed: bool,
}

/// Event store using WAL
pub struct EventStore {
    wal: Wal,
    next_event_id: u64,
}

impl EventStore {
    /// Opens or creates an event store
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();

        let config = WalConfig {
            dir: path.join("events"),
            max_segment_size: 128 * 1024 * 1024,
            fsync_policy: nori_wal::FsyncPolicy::Batch(
                std::time::Duration::from_millis(5)
            ),
            preallocate: true,
            node_id: 0,
        };

        let (wal, recovery_info) = Wal::open(config).await?;

        println!("Event store recovered:");
        println!("  Events: {}", recovery_info.valid_records);
        println!("  Segments: {}", recovery_info.segments_scanned);

        // Find highest event ID
        let mut next_event_id = 0u64;
        let mut reader = wal.read_from(Position { segment_id: 0, offset: 0 }).await?;

        while let Some((record, _)) = reader.next_record().await? {
            if let Ok(event) = serde_json::from_slice::<Event>(&record.value) {
                next_event_id = next_event_id.max(event.event_id + 1);
            }
        }

        println!("  Next event ID: {}", next_event_id);

        Ok(Self { wal, next_event_id })
    }

    /// Appends an event to the store
    pub async fn append_event(
        &mut self,
        event_type: String,
        payload: Bytes,
    ) -> Result<u64> {
        let event = Event {
            event_id: self.next_event_id,
            event_type,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
            payload,
        };

        // Serialize event
        let event_bytes = serde_json::to_vec(&event)?;

        // Write to WAL
        let record = Record::put(&event.event_id.to_le_bytes(), &event_bytes);
        self.wal.append(&record).await?;

        let event_id = self.next_event_id;
        self.next_event_id += 1;

        Ok(event_id)
    }

    /// Syncs events to disk
    pub async fn sync(&self) -> Result<()> {
        self.wal.sync().await?;
        Ok(())
    }

    /// Reads all events from the store
    pub async fn read_all_events(&self) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        let mut reader = self.wal.read_from(Position { segment_id: 0, offset: 0 }).await?;

        while let Some((record, _)) = reader.next_record().await? {
            if let Ok(event) = serde_json::from_slice::<Event>(&record.value) {
                events.push(event);
            }
        }

        Ok(events)
    }

    /// Reads events from a specific position
    pub async fn read_from(&self, from_event_id: u64) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        let mut reader = self.wal.read_from(Position { segment_id: 0, offset: 0 }).await?;

        while let Some((record, _)) = reader.next_record().await? {
            if let Ok(event) = serde_json::from_slice::<Event>(&record.value) {
                if event.event_id >= from_event_id {
                    events.push(event);
                }
            }
        }

        Ok(events)
    }

    /// Gracefully closes the event store
    pub async fn close(self) -> Result<()> {
        self.wal.close().await?;
        Ok(())
    }
}

/// Application service using event sourcing
pub struct AccountService {
    event_store: EventStore,
}

impl AccountService {
    pub async fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let event_store = EventStore::open(path).await?;
        Ok(Self { event_store })
    }

    /// Opens a new account
    pub async fn open_account(&mut self, account_id: String, initial_balance: i64) -> Result<u64> {
        let event = AccountEvent::AccountOpened {
            account_id,
            initial_balance,
        };

        let payload = serde_json::to_vec(&event)?;
        let event_id = self.event_store.append_event(
            "AccountOpened".to_string(),
            Bytes::from(payload),
        ).await?;

        Ok(event_id)
    }

    /// Deposits money
    pub async fn deposit(&mut self, account_id: String, amount: i64) -> Result<u64> {
        let event = AccountEvent::MoneyDeposited {
            account_id,
            amount,
        };

        let payload = serde_json::to_vec(&event)?;
        let event_id = self.event_store.append_event(
            "MoneyDeposited".to_string(),
            Bytes::from(payload),
        ).await?;

        Ok(event_id)
    }

    /// Withdraws money
    pub async fn withdraw(&mut self, account_id: String, amount: i64) -> Result<u64> {
        // First, check current balance by replaying events
        let account = self.get_account_state(&account_id).await?;

        if account.balance < amount {
            return Err(anyhow::anyhow!("Insufficient balance"));
        }

        let event = AccountEvent::MoneyWithdrawn {
            account_id,
            amount,
        };

        let payload = serde_json::to_vec(&event)?;
        let event_id = self.event_store.append_event(
            "MoneyWithdrawn".to_string(),
            Bytes::from(payload),
        ).await?;

        Ok(event_id)
    }

    /// Gets account state by replaying events
    pub async fn get_account_state(&self, account_id: &str) -> Result<Account> {
        let events = self.event_store.read_all_events().await?;

        let mut account = Account {
            id: account_id.to_string(),
            balance: 0,
            is_closed: false,
        };

        for event in events {
            if let Ok(account_event) = serde_json::from_slice::<AccountEvent>(&event.payload) {
                match account_event {
                    AccountEvent::AccountOpened { account_id: id, initial_balance } => {
                        if id == account_id {
                            account.balance = initial_balance;
                        }
                    }
                    AccountEvent::MoneyDeposited { account_id: id, amount } => {
                        if id == account_id {
                            account.balance += amount;
                        }
                    }
                    AccountEvent::MoneyWithdrawn { account_id: id, amount } => {
                        if id == account_id {
                            account.balance -= amount;
                        }
                    }
                    AccountEvent::AccountClosed { account_id: id } => {
                        if id == account_id {
                            account.is_closed = true;
                        }
                    }
                }
            }
        }

        Ok(account)
    }

    /// Syncs events to disk
    pub async fn sync(&self) -> Result<()> {
        self.event_store.sync().await
    }

    /// Closes the service
    pub async fn close(self) -> Result<()> {
        self.event_store.close().await
    }
}

// Example usage
#[tokio::main]
async fn main() -> Result<()> {
    let mut service = AccountService::new("./event_store").await?;

    // Open account
    service.open_account("alice".to_string(), 1000).await?;

    // Perform operations
    service.deposit("alice".to_string(), 500).await?;
    service.withdraw("alice".to_string(), 200).await?;

    // Sync to ensure durability
    service.sync().await?;

    // Query current state
    let account = service.get_account_state("alice").await?;
    println!("Alice's balance: ${}", account.balance);

    // Close service
    service.close().await?;

    Ok(())
}
```

## How It Works

### 1. Event Storage

Events are stored as WAL records:

```rust
pub async fn append_event(&mut self, event_type: String, payload: Bytes) -> Result<u64> {
    let event = Event {
        event_id: self.next_event_id,
        event_type,
        timestamp: current_timestamp(),
        payload,
    };

    let event_bytes = serde_json::to_vec(&event)?;
    let record = Record::put(&event.event_id.to_le_bytes(), &event_bytes);
    self.wal.append(&record).await?;

    self.next_event_id += 1;
    Ok(event.event_id)
}
```

### 2. State Reconstruction

State is derived by replaying all events:

```rust
pub async fn get_account_state(&self, account_id: &str) -> Result<Account> {
    let events = self.event_store.read_all_events().await?;

    let mut account = Account::default();

    for event in events {
        // Apply event to state
        match event {
            AccountEvent::AccountOpened { initial_balance, .. } => {
                account.balance = initial_balance;
            }
            AccountEvent::MoneyDeposited { amount, .. } => {
                account.balance += amount;
            }
            // ... other events
        }
    }

    Ok(account)
}
```

### 3. Command Validation

Commands validate against current state before appending events:

```rust
pub async fn withdraw(&mut self, account_id: String, amount: i64) -> Result<u64> {
    // 1. Rebuild state
    let account = self.get_account_state(&account_id).await?;

    // 2. Validate command
    if account.balance < amount {
        return Err(anyhow::anyhow!("Insufficient balance"));
    }

    // 3. Append event
    let event = AccountEvent::MoneyWithdrawn { account_id, amount };
    self.event_store.append_event(event).await
}
```

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_event_sourcing_basic() {
        let dir = TempDir::new().unwrap();
        let mut service = AccountService::new(dir.path()).await.unwrap();

        // Open account
        service.open_account("alice".to_string(), 1000).await.unwrap();

        // Deposit
        service.deposit("alice".to_string(), 500).await.unwrap();

        // Check state
        let account = service.get_account_state("alice").await.unwrap();
        assert_eq!(account.balance, 1500);
    }

    #[tokio::test]
    async fn test_event_replay() {
        let dir = TempDir::new().unwrap();

        // Write events
        {
            let mut service = AccountService::new(dir.path()).await.unwrap();
            service.open_account("bob".to_string(), 2000).await.unwrap();
            service.deposit("bob".to_string(), 1000).await.unwrap();
            service.withdraw("bob".to_string(), 500).await.unwrap();
            service.sync().await.unwrap();
        }

        // Reopen and verify state is reconstructed
        {
            let service = AccountService::new(dir.path()).await.unwrap();
            let account = service.get_account_state("bob").await.unwrap();
            assert_eq!(account.balance, 2500);
        }
    }

    #[tokio::test]
    async fn test_insufficient_balance() {
        let dir = TempDir::new().unwrap();
        let mut service = AccountService::new(dir.path()).await.unwrap();

        service.open_account("charlie".to_string(), 100).await.unwrap();

        // Try to withdraw more than balance
        let result = service.withdraw("charlie".to_string(), 200).await;
        assert!(result.is_err());
    }
}
```

## Production Considerations

### 1. Snapshots

Replaying millions of events is slow. Add snapshots:

```rust
pub struct EventStore {
    wal: Wal,
    next_event_id: u64,
    snapshot_dir: PathBuf,
}

impl EventStore {
    /// Creates a snapshot at current position
    pub async fn create_snapshot(&self, state: &impl Serialize) -> Result<u64> {
        let snapshot_id = self.next_event_id - 1;
        let snapshot_path = self.snapshot_dir.join(format!("{}.snapshot", snapshot_id));

        let snapshot_bytes = serde_json::to_vec(state)?;
        tokio::fs::write(&snapshot_path, snapshot_bytes).await?;

        Ok(snapshot_id)
    }

    /// Loads latest snapshot
    pub async fn load_snapshot<T: DeserializeOwned>(&self) -> Result<Option<(u64, T)>> {
        let mut snapshots = Vec::new();

        let mut entries = tokio::fs::read_dir(&self.snapshot_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(id_str) = name.strip_suffix(".snapshot") {
                    if let Ok(id) = id_str.parse::<u64>() {
                        snapshots.push(id);
                    }
                }
            }
        }

        if snapshots.is_empty() {
            return Ok(None);
        }

        // Load most recent snapshot
        snapshots.sort();
        let latest_id = snapshots.last().unwrap();
        let snapshot_path = self.snapshot_dir.join(format!("{}.snapshot", latest_id));

        let snapshot_bytes = tokio::fs::read(&snapshot_path).await?;
        let state = serde_json::from_slice(&snapshot_bytes)?;

        Ok(Some((*latest_id, state)))
    }
}
```

### 2. Event Versioning

Events evolve over time. Use versioning:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccountEventV1 {
    AccountOpened { account_id: String, initial_balance: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccountEventV2 {
    AccountOpened { account_id: String, initial_balance: i64, currency: String },
    MoneyDeposited { account_id: String, amount: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum VersionedEvent {
    V1(AccountEventV1),
    V2(AccountEventV2),
}

// Upcasting from V1 to V2
impl From<AccountEventV1> for AccountEventV2 {
    fn from(v1: AccountEventV1) -> Self {
        match v1 {
            AccountEventV1::AccountOpened { account_id, initial_balance } => {
                AccountEventV2::AccountOpened {
                    account_id,
                    initial_balance,
                    currency: "USD".to_string(), // Default for old events
                }
            }
        }
    }
}
```

### 3. Projections

Maintain read models for fast queries:

```rust
pub struct AccountProjection {
    accounts: HashMap<String, Account>,
    last_processed_event: u64,
}

impl AccountProjection {
    pub async fn update_from_events(&mut self, events: Vec<Event>) -> Result<()> {
        for event in events {
            if event.event_id <= self.last_processed_event {
                continue; // Already processed
            }

            // Apply event to projection
            if let Ok(account_event) = serde_json::from_slice::<AccountEvent>(&event.payload) {
                self.apply_event(account_event);
            }

            self.last_processed_event = event.event_id;
        }

        Ok(())
    }

    fn apply_event(&mut self, event: AccountEvent) {
        // Update in-memory projection
        match event {
            AccountEvent::AccountOpened { account_id, initial_balance } => {
                self.accounts.insert(account_id.clone(), Account {
                    id: account_id,
                    balance: initial_balance,
                    is_closed: false,
                });
            }
            // ... other events
        }
    }

    pub fn get_account(&self, account_id: &str) -> Option<&Account> {
        self.accounts.get(account_id)
    }
}
```

### 4. Monitoring

Track event store metrics:

```rust
// Events appended
metrics.counter("events.appended", 1);

// Event types
metrics.counter("events.type", 1, &[("type", event.event_type)]);

// Replay time
let start = Instant::now();
let events = store.read_all_events().await?;
metrics.histogram("events.replay_ms", start.elapsed().as_millis());
```

## Enhancements

### Time Travel Queries

Query state at any point in time:

```rust
pub async fn get_account_state_at(
    &self,
    account_id: &str,
    timestamp: u64,
) -> Result<Account> {
    let events = self.event_store.read_all_events().await?;

    let mut account = Account::default();

    for event in events {
        if event.timestamp > timestamp {
            break; // Stop at target time
        }

        // Apply event
        // ...
    }

    Ok(account)
}
```

### Event Subscriptions

Stream events to subscribers:

```rust
pub struct EventSubscriber {
    store: EventStore,
    last_seen_event: u64,
}

impl EventSubscriber {
    pub async fn poll_new_events(&mut self) -> Result<Vec<Event>> {
        let events = self.store.read_from(self.last_seen_event + 1).await?;

        if let Some(last) = events.last() {
            self.last_seen_event = last.event_id;
        }

        Ok(events)
    }
}
```

## Conclusion

This recipe demonstrates:
- Using WAL for event storage
- Rebuilding state from events
- Command validation
- Event replay and recovery

For distributed event sourcing, combine with [Replication recipe](replication.md).
