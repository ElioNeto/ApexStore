use crate::infra::error::Result;
use serde::{de::DeserializeOwned, Serialize};

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    Ok(postcard::to_allocvec(value)?)
}

pub fn decode<T: DeserializeOwned>(data: &[u8]) -> Result<T> {
    Ok(postcard::from_bytes(data)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn test_encode_decode_roundtrip() {
        let original = vec![1u8, 2, 3, 4, 5];
        let encoded = encode(&original).unwrap();
        let decoded: Vec<u8> = decode(&encoded).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_encode_decode_string() {
        let original = "hello apexstore";
        let encoded = encode(&original).unwrap();
        let decoded: String = decode(&encoded).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_encode_decode_struct() {
        #[derive(Serialize, Deserialize, Debug, PartialEq)]
        struct Point {
            x: f64,
            y: f64,
            label: String,
        }

        let original = Point {
            x: 1.5,
            y: -3.2,
            label: "origin".into(),
        };
        let encoded = encode(&original).unwrap();
        let decoded: Point = decode(&encoded).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_decode_empty_data() {
        let result: Result<Vec<u8>> = decode(b"");
        assert!(result.is_err());
    }
}
