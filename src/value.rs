#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Data(Vec<u8>),
    Tombstone,
    // other variants …
}

impl Value {
    /// Returns true if this value represents a tombstone (deletion marker).
    pub fn is_tombstone(&self) -> bool {
        matches!(self, Value::Tombstone)
    }
}
