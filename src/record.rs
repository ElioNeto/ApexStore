#[derive(Clone, Debug)]
pub struct Record {
    pub key: Vec<u8>,
    pub value: crate::value::Value,
}

impl Record {
    pub fn new(key: Vec<u8>, value: crate::value::Value) -> Self {
        Self { key, value }
    }
}
