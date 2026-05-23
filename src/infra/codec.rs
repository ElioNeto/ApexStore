use crate::infra::error::Result;
use serde::{de::DeserializeOwned, Serialize};

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    Ok(postcard::to_allocvec(value)?)
}

pub fn decode<T: DeserializeOwned>(data: &[u8]) -> Result<T> {
    Ok(postcard::from_bytes(data)?)
}
