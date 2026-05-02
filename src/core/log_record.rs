use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogRecord {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub timestamp: u128,
    pub is_deleted: bool,
}

impl LogRecord {
    pub fn new(key: Vec<u8>, value: Vec<u8>) -> Self {
        Self {
            key,
            value,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            is_deleted: false,
        }
    }

    pub fn tombstone(key: Vec<u8>) -> Self {
        Self {
            key,
            value: Vec::new(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            is_deleted: true,
        }
    }
}
