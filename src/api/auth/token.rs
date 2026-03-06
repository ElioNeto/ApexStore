use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub id: String,
    pub name: String,
    pub value: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl Token {
    pub fn new(name: String, expiry_days: Option<u32>) -> Self {
        let id = Uuid::new_v4().to_string();
        let value = generate_token_value();
        let created_at = Utc::now();
        let expires_at = expiry_days.map(|days| created_at + Duration::days(days as i64));

        Self {
            id,
            name,
            value,
            created_at,
            expires_at,
        }
    }

    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            Utc::now() > expires_at
        } else {
            false
        }
    }

    /// Sanitized version without sensitive value
    pub fn sanitized(&self) -> SanitizedToken {
        SanitizedToken {
            id: self.id.clone(),
            name: self.name.clone(),
            created_at: self.created_at,
            expires_at: self.expires_at,
            value_prefix: self.value.chars().take(10).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanitizedToken {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub value_prefix: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateTokenResponse {
    pub token: Token,
}

#[derive(Debug, Serialize)]
pub struct TokenListResponse {
    pub tokens: Vec<SanitizedToken>,
}

/// Generate a secure random token value
fn generate_token_value() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let random_bytes: Vec<u8> = (0..32).map(|_| rng.gen::<u8>()).collect();

    // Use base64 engine for encoding
    use base64::{engine::general_purpose, Engine as _};
    format!("apx_{}", general_purpose::STANDARD.encode(&random_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_creation() {
        let token = Token::new("test".to_string(), Some(30));

        assert!(!token.id.is_empty());
        assert_eq!(token.name, "test");
        assert!(token.value.starts_with("apx_"));
        assert!(token.expires_at.is_some());
        assert!(!token.is_expired());
    }

    #[test]
    fn test_token_no_expiry() {
        let token = Token::new("test".to_string(), None);

        assert!(token.expires_at.is_none());
        assert!(!token.is_expired());
    }

    #[test]
    fn test_sanitized_token() {
        let token = Token::new("test".to_string(), Some(30));
        let sanitized = token.sanitized();

        assert_eq!(sanitized.id, token.id);
        assert_eq!(sanitized.name, token.name);
        assert_eq!(sanitized.value_prefix.len(), 10);
        assert!(token.value.starts_with(&sanitized.value_prefix));
    }
}
