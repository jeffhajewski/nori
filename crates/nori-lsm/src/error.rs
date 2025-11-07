use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WAL error: {0}")]
    Wal(#[from] nori_wal::RecordError),

    #[error("SSTable error: {0}")]
    SSTable(String),

    #[error("MANIFEST error: {0}")]
    Manifest(String),

    #[error("Invalid configuration: {0}")]
    Config(String),

    #[error("Compaction error: {0}")]
    Compaction(String),

    #[error("Memtable full")]
    MemtableFull,

    #[error("L0 stall: too many files ({0} > {1})")]
    L0Stall(usize, usize),

    #[error("System pressure critical: {0}")]
    SystemPressure(String),

    #[error("Key not found")]
    KeyNotFound,

    #[error("Slot not found: level={0}, slot_id={1}")]
    SlotNotFound(u8, u32),

    #[error("Guard invariant violated: {0}")]
    GuardInvariant(String),

    #[error("Sequence number overflow")]
    SeqnoOverflow,

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;
