# Implementation Plan - Issues #31 & #36

**Branch**: `feature/31-36-auth-concurrent`  
**Issues**: 
- [#36 - Enable Concurrent Reads in SstableReader](https://github.com/ElioNeto/lsm-kv-store/issues/36)
- [#31 - Implement Bearer Token Authentication for API Layer](https://github.com/ElioNeto/lsm-kv-store/issues/31)

**Status**: Planning  
**Created**: 2026-02-04  
**Estimated Effort**: 5-8 days

---

## 🎯 Objectives

### Issue #36: Concurrent Reads (Storage Layer)
- Enable multiple threads to read from the same SSTable simultaneously
- Change `SstableReader::get()` and `scan()` from `&mut self` to `&self`
- Wrap file operations in thread-safe constructs
- Add concurrency tests

### Issue #31: Bearer Token Authentication (API Layer)
- Implement Bearer Token authentication for REST API
- Protect all endpoints except `/health`
- Add token management endpoints (create, list, revoke)
- Store tokens securely (hashed with SHA-256)

---

## 🏗️ Implementation Strategy

**Order**: Phase 1 (#36) → Phase 2 (#31)

Rationale: Optimize the storage engine first, then add security layer on top.

---

## Phase 1: Concurrent Reads (#36) - Storage Layer

### 1.1 Update `SstableReader` for Thread-Safety

**File**: `src/storage/reader.rs`

#### Changes:

```rust
use std::sync::Mutex;

pub struct SstableReader {
    metadata: MetaBlock,
    bloom_filter: Bloom<[u8]>,
    file: Mutex<File>,                    // ← NEW: Thread-safe file access
    block_cache: Arc<GlobalBlockCache>,   // ← Already thread-safe (from #35)
    path: PathBuf,
    config: StorageConfig,
}

impl SstableReader {
    pub fn open(
        path: PathBuf,
        config: StorageConfig,
        block_cache: Arc<GlobalBlockCache>,
    ) -> Result<Self> {
        let file = File::open(&path)?;
        // ... load metadata, bloom filter
        
        Ok(Self {
            metadata,
            bloom_filter,
            file: Mutex::new(file),  // ← Wrap in Mutex
            block_cache,
            path,
            config,
        })
    }

    // Change from &mut self to &self
    pub fn get(&self, key: &str) -> Result<Option<LogRecord>> {
        // 1. Bloom filter check (no lock needed - immutable)
        if !self.might_contain(key) {
            return Ok(None);
        }

        // 2. Binary search on metadata (no lock - immutable)
        let block_idx = self.find_block(key)?;
        if block_idx.is_none() {
            return Ok(None);
        }

        // 3. Check cache or read from disk
        let block = self.read_block(block_idx.unwrap())?;

        // 4. Search within block (no lock - in-memory)
        self.search_in_block(&block, key.as_bytes())
    }

    fn read_block(&self, idx: usize) -> Result<Block> {
        let cache_key = CacheKey::new(&self.path, self.metadata.blocks[idx].offset);

        // Try cache first
        if let Some(data) = self.block_cache.get(&cache_key) {
            return Block::decode(&data);
        }

        // Cache miss - read from disk with file lock
        let data = {
            let mut file = self.file.lock().unwrap();
            file.seek(SeekFrom::Start(self.metadata.blocks[idx].offset))?;
            let mut buf = vec![0u8; self.metadata.blocks[idx].size as usize];
            file.read_exact(&mut buf)?;
            buf
        }; // Lock released here

        // Store in cache
        self.block_cache.put(cache_key, data.clone());

        Block::decode(&data)
    }

    // Change from &mut self to &self
    pub fn scan(&self) -> Result<Vec<(Vec<u8>, LogRecord)>> {
        let mut results = Vec::new();

        for block_meta in &self.metadata.blocks {
            let block = self.read_block_at_offset(block_meta.offset)?;
            
            for &offset in &block.offsets {
                let (key, record) = self.decode_entry(&block.data, offset as usize)?;
                results.push((key, record));
            }
        }

        Ok(results)
    }
}
```

#### Key Changes:
- `file: File` → `file: Mutex<File>`
- All methods take `&self` instead of `&mut self`
- File lock held only during disk I/O (minimal critical section)
- Cache and metadata access remain lock-free

#### Testing:

**File**: `src/storage/reader.rs` (add at bottom)

```rust
#[cfg(test)]
mod concurrency_tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_concurrent_reads_same_sstable() {
        // Setup: Create SSTable with 100 keys
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("concurrent_test.sst");
        let config = StorageConfig::default();
        let cache = GlobalBlockCache::new(10, config.block_size);

        // Build SSTable
        let mut builder = SstableBuilder::new(path.clone(), config.clone(), 1).unwrap();
        for i in 0..100 {
            let key = format!("key_{:03}", i);
            let record = LogRecord::new(key.clone(), format!("value_{:03}", i).into_bytes());
            builder.add(key.as_bytes(), &record).unwrap();
        }
        builder.finish().unwrap();

        // Test: 10 threads reading concurrently
        let reader = Arc::new(SstableReader::open(path, config, cache).unwrap());
        let handles: Vec<_> = (0..10)
            .map(|thread_id| {
                let r = Arc::clone(&reader);
                thread::spawn(move || {
                    for i in 0..100 {
                        let key = format!("key_{:03}", (thread_id * 10 + i) % 100);
                        let result = r.get(&key).unwrap();
                        assert!(result.is_some(), "Key {} should exist", key);
                    }
                })
            })
            .collect();

        // Wait for all threads
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_get_and_scan() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scan_test.sst");
        let config = StorageConfig::default();
        let cache = GlobalBlockCache::new(10, config.block_size);

        // Build SSTable
        let mut builder = SstableBuilder::new(path.clone(), config.clone(), 1).unwrap();
        for i in 0..50 {
            let key = format!("key_{:03}", i);
            let record = LogRecord::new(key.clone(), vec![i as u8]);
            builder.add(key.as_bytes(), &record).unwrap();
        }
        builder.finish().unwrap();

        let reader = Arc::new(SstableReader::open(path, config, cache).unwrap());

        // 5 threads doing get, 2 threads doing scan
        let mut handles = vec![];

        for _ in 0..5 {
            let r = Arc::clone(&reader);
            handles.push(thread::spawn(move || {
                for i in 0..50 {
                    r.get(&format!("key_{:03}", i)).unwrap();
                }
            }));
        }

        for _ in 0..2 {
            let r = Arc::clone(&reader);
            handles.push(thread::spawn(move || {
                let results = r.scan().unwrap();
                assert_eq!(results.len(), 50);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }
}
```

### 1.2 Update `LsmEngine`

**File**: `src/core/engine.rs`

#### Changes:

```rust
impl LsmEngine {
    // Change signature: &mut self → &self for read operations
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        // 1. Check MemTable (with read lock)
        {
            let memtable = self.memtable.read().unwrap();
            if let Some(record) = memtable.get(key) {
                if record.is_tombstone() {
                    return Ok(None);
                }
                return Ok(Some(record.value.clone()));
            }
        } // Read lock released

        // 2. Check SSTables (newest to oldest)
        let sstables = self.sstables.read().unwrap();
        for sstable in sstables.iter().rev() {
            if let Some(record) = sstable.get(key)? {  // ← Now using &self
                if record.is_tombstone() {
                    return Ok(None);
                }
                return Ok(Some(record.value));
            }
        }

        Ok(None)
    }

    // scan and range_scan can also use &self now
    pub fn scan(&self, start: Option<&str>, end: Option<&str>) -> Result<Vec<(String, Vec<u8>)>> {
        // Implementation using &self
    }
}
```

### 1.3 Update Existing Tests

**File**: `tests/integration_sstable_v2.rs`

Search and replace:
- `let mut reader = SstableReader::open(...)` → `let reader = SstableReader::open(...)`
- `reader.get(...)` (no more `&mut` needed)

---

## Phase 2: Bearer Token Authentication (#31) - API Layer

### 2.1 Create Auth Module Structure

**Files to create**:
```
src/api/auth/
├── mod.rs
├── token.rs
├── middleware.rs
├── manager.rs
└── error.rs
```

### 2.2 Token System Implementation

**File**: `src/api/auth/token.rs`

```rust
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: Uuid,
    pub name: String,
    pub token_hash: String,
    pub created_at: u128,
    pub expires_at: Option<u128>,
    pub permissions: Vec<Permission>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Permission {
    Read,
    Write,
    Delete,
    Admin,
}

impl ApiToken {
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis();
            return now > expires_at;
        }
        false
    }

    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.permissions.contains(perm) || self.permissions.contains(&Permission::Admin)
    }
}

pub fn generate_token(name: &str) -> String {
    use rand::Rng;
    let random: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();
    format!("lsm_{}", random)
}

pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn validate_token_hash(token: &str, expected_hash: &str) -> bool {
    use std::time::Duration;
    // Constant-time comparison to prevent timing attacks
    let computed = hash_token(token);
    if computed.len() != expected_hash.len() {
        return false;
    }
    
    let mut result = 0u8;
    for (a, b) in computed.bytes().zip(expected_hash.bytes()) {
        result |= a ^ b;
    }
    result == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_generation() {
        let token = generate_token("test-token");
        assert!(token.starts_with("lsm_"));
        assert_eq!(token.len(), 68); // "lsm_" + 64 chars
    }

    #[test]
    fn test_token_hash_validation() {
        let token = generate_token("test");
        let hash = hash_token(&token);
        assert!(validate_token_hash(&token, &hash));
        assert!(!validate_token_hash("wrong_token", &hash));
    }

    #[test]
    fn test_token_expiry() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();

        let expired = ApiToken {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            token_hash: "hash".to_string(),
            created_at: now - 10000,
            expires_at: Some(now - 1000),
            permissions: vec![],
        };
        assert!(expired.is_expired());

        let valid = ApiToken {
            expires_at: Some(now + 10000),
            ..expired.clone()
        };
        assert!(!valid.is_expired());
    }

    #[test]
    fn test_permissions() {
        let token = ApiToken {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            token_hash: "hash".to_string(),
            created_at: 0,
            expires_at: None,
            permissions: vec![Permission::Read, Permission::Write],
        };

        assert!(token.has_permission(&Permission::Read));
        assert!(token.has_permission(&Permission::Write));
        assert!(!token.has_permission(&Permission::Delete));

        let admin_token = ApiToken {
            permissions: vec![Permission::Admin],
            ..token
        };
        assert!(admin_token.has_permission(&Permission::Delete));
    }
}
```

**File**: `src/api/auth/error.rs`

```rust
use actix_web::{HttpResponse, ResponseError};
use std::fmt;

#[derive(Debug)]
pub enum AuthError {
    InvalidToken,
    ExpiredToken,
    MissingToken,
    InsufficientPermissions,
    TokenNotFound,
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::InvalidToken => write!(f, "Invalid token"),
            AuthError::ExpiredToken => write!(f, "Token has expired"),
            AuthError::MissingToken => write!(f, "Missing authorization token"),
            AuthError::InsufficientPermissions => write!(f, "Insufficient permissions"),
            AuthError::TokenNotFound => write!(f, "Token not found"),
        }
    }
}

impl ResponseError for AuthError {
    fn error_response(&self) -> HttpResponse {
        match self {
            AuthError::InvalidToken | AuthError::ExpiredToken | AuthError::MissingToken => {
                HttpResponse::Unauthorized().json(serde_json::json!({
                    "error": self.to_string()
                }))
            }
            AuthError::InsufficientPermissions => HttpResponse::Forbidden().json(serde_json::json!({
                "error": self.to_string()
            })),
            AuthError::TokenNotFound => HttpResponse::NotFound().json(serde_json::json!({
                "error": self.to_string()
            })),
        }
    }
}
```

**File**: `src/api/auth/manager.rs`

```rust
use super::{error::AuthError, token::*};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

pub struct TokenManager {
    tokens: RwLock<HashMap<Uuid, ApiToken>>,
}

impl TokenManager {
    pub fn new() -> Self {
        Self {
            tokens: RwLock::new(HashMap::new()),
        }
    }

    pub fn create(
        &self,
        name: String,
        permissions: Vec<Permission>,
        expiry_days: Option<u32>,
    ) -> Result<(String, ApiToken), AuthError> {
        let token_str = generate_token(&name);
        let token_hash = hash_token(&token_str);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();

        let expires_at = expiry_days.map(|days| {
            now + (days as u128 * 24 * 60 * 60 * 1000)
        });

        let api_token = ApiToken {
            id: Uuid::new_v4(),
            name,
            token_hash,
            created_at: now,
            expires_at,
            permissions,
        };

        let mut tokens = self.tokens.write().unwrap();
        tokens.insert(api_token.id, api_token.clone());

        Ok((token_str, api_token))
    }

    pub fn validate(&self, token: &str) -> Result<ApiToken, AuthError> {
        let tokens = self.tokens.read().unwrap();

        for api_token in tokens.values() {
            if validate_token_hash(token, &api_token.token_hash) {
                if api_token.is_expired() {
                    return Err(AuthError::ExpiredToken);
                }
                return Ok(api_token.clone());
            }
        }

        Err(AuthError::InvalidToken)
    }

    pub fn revoke(&self, id: Uuid) -> Result<(), AuthError> {
        let mut tokens = self.tokens.write().unwrap();
        tokens.remove(&id).ok_or(AuthError::TokenNotFound)?;
        Ok(())
    }

    pub fn list(&self) -> Vec<ApiToken> {
        let tokens = self.tokens.read().unwrap();
        tokens.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_lifecycle() {
        let manager = TokenManager::new();

        // Create
        let (token, api_token) = manager
            .create("test-token".to_string(), vec![Permission::Read], None)
            .unwrap();
        assert_eq!(api_token.name, "test-token");

        // Validate
        let validated = manager.validate(&token).unwrap();
        assert_eq!(validated.id, api_token.id);

        // List
        let tokens = manager.list();
        assert_eq!(tokens.len(), 1);

        // Revoke
        manager.revoke(api_token.id).unwrap();
        assert!(manager.validate(&token).is_err());
    }
}
```

**File**: `src/api/auth/middleware.rs`

```rust
use super::{error::AuthError, manager::TokenManager};
use actix_web::{dev::ServiceRequest, Error, HttpMessage};
use actix_web_httpauth::extractors::bearer::BearerAuth;
use std::sync::Arc;

pub async fn validator(
    req: ServiceRequest,
    credentials: BearerAuth,
    token_manager: Arc<TokenManager>,
) -> Result<ServiceRequest, (Error, ServiceRequest)> {
    let token = credentials.token();

    match token_manager.validate(token) {
        Ok(api_token) => {
            req.extensions_mut().insert(api_token);
            Ok(req)
        }
        Err(e) => Err((e.into(), req)),
    }
}
```

**File**: `src/api/auth/mod.rs`

```rust
pub mod error;
pub mod manager;
pub mod middleware;
pub mod token;

pub use error::AuthError;
pub use manager::TokenManager;
pub use token::{ApiToken, Permission};
```

### 2.3 Configuration Updates

**File**: `.env` (add)

```bash
# Authentication
API_AUTH_ENABLED=true
API_AUTH_SECRET=change-me-in-production-use-random-string
API_TOKEN_EXPIRY_DAYS=30
```

**File**: `src/infra/config.rs` (update)

```rust
use dotenvy::dotenv;
use std::env;

#[derive(Clone, Debug)]
pub struct ApiConfig {
    pub host: String,
    pub port: u16,
    pub auth_enabled: bool,
    pub auth_secret: String,
    pub token_expiry_days: u32,
}

impl ApiConfig {
    pub fn from_env() -> Self {
        dotenv().ok();
        
        Self {
            host: env::var("API_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: env::var("API_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .expect("API_PORT must be a number"),
            auth_enabled: env::var("API_AUTH_ENABLED")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),
            auth_secret: env::var("API_AUTH_SECRET")
                .unwrap_or_else(|_| "default-secret-change-me".to_string()),
            token_expiry_days: env::var("API_TOKEN_EXPIRY_DAYS")
                .unwrap_or_else(|_| "30".to_string())
                .parse()
                .unwrap_or(30),
        }
    }
}
```

### 2.4 Route Integration

**File**: `src/api/handlers.rs` (add token management endpoints)

```rust
use crate::api::auth::{manager::TokenManager, token::Permission, ApiToken};
use actix_web::{web, HttpResponse, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    pub permissions: Vec<Permission>,
    pub expiry_days: Option<u32>,
}

#[derive(Serialize)]
pub struct CreateTokenResponse {
    pub token: String,
    pub id: Uuid,
    pub name: String,
    pub permissions: Vec<Permission>,
}

pub async fn create_token(
    body: web::Json<CreateTokenRequest>,
    token_manager: web::Data<Arc<TokenManager>>,
) -> Result<HttpResponse> {
    let (token, api_token) = token_manager
        .create(
            body.name.clone(),
            body.permissions.clone(),
            body.expiry_days,
        )
        .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(HttpResponse::Ok().json(CreateTokenResponse {
        token,
        id: api_token.id,
        name: api_token.name,
        permissions: api_token.permissions,
    }))
}

pub async fn list_tokens(
    token_manager: web::Data<Arc<TokenManager>>,
) -> Result<HttpResponse> {
    let tokens = token_manager.list();
    Ok(HttpResponse::Ok().json(tokens))
}

pub async fn revoke_token(
    path: web::Path<Uuid>,
    token_manager: web::Data<Arc<TokenManager>>,
) -> Result<HttpResponse> {
    token_manager
        .revoke(*path)
        .map_err(|e| actix_web::error::ErrorNotFound(e))?;
    
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Token revoked successfully"
    })))
}
```

**File**: `src/api/server.rs` (update)

```rust
use crate::api::auth::{manager::TokenManager, middleware::validator};
use crate::api::handlers::*;
use crate::core::engine::LsmEngine;
use crate::infra::config::ApiConfig;
use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpServer};
use actix_web_httpauth::middleware::HttpAuthentication;
use std::sync::Arc;

pub async fn start_server(
    engine: Arc<LsmEngine>,
    config: ApiConfig,
) -> std::io::Result<()> {
    let token_manager = Arc::new(TokenManager::new());
    let addr = format!("{}:{}", config.host, config.port);

    println!("🚀 Starting LSM-KV API Server on {}", addr);
    println!("📝 Authentication: {}", if config.auth_enabled { "ENABLED" } else { "DISABLED" });

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();

        let mut app = App::new()
            .wrap(cors)
            .wrap(Logger::default())
            .app_data(web::Data::new(engine.clone()))
            .app_data(web::Data::new(token_manager.clone()))
            .app_data(web::Data::new(config.clone()));

        // Public endpoints
        app = app.route("/health", web::get().to(health_check));

        // Protected endpoints (conditionally wrap with auth)
        if config.auth_enabled {
            let token_manager_clone = token_manager.clone();
            let auth = HttpAuthentication::bearer(move |req, cred| {
                validator(req, cred, token_manager_clone.clone())
            });

            app = app.service(
                web::scope("")
                    .wrap(auth)
                    .route("/keys", web::post().to(put_key))
                    .route("/keys/{key}", web::get().to(get_key))
                    .route("/keys/{key}", web::delete().to(delete_key))
                    .route("/stats/all", web::get().to(get_all_stats))
                    .route("/admin/tokens", web::post().to(create_token))
                    .route("/admin/tokens", web::get().to(list_tokens))
                    .route("/admin/tokens/{id}", web::delete().to(revoke_token)),
            );
        } else {
            // No auth - all endpoints open
            app = app
                .route("/keys", web::post().to(put_key))
                .route("/keys/{key}", web::get().to(get_key))
                .route("/keys/{key}", web::delete().to(delete_key))
                .route("/stats/all", web::get().to(get_all_stats))
                .route("/admin/tokens", web::post().to(create_token))
                .route("/admin/tokens", web::get().to(list_tokens))
                .route("/admin/tokens/{id}", web::delete().to(revoke_token));
        }

        app
    })
    .bind(&addr)?
    .run()
    .await
}
```

### 2.5 Integration Tests

**File**: `tests/integration_auth.rs` (new)

```rust
use actix_web::{test, App};
use lsm_kv_store::api::auth::TokenManager;
use lsm_kv_store::api::server::configure_routes;
use std::sync::Arc;

#[actix_web::test]
async fn test_health_endpoint_public() {
    let token_manager = Arc::new(TokenManager::new());
    let app = test::init_service(
        App::new()
            .app_data(actix_web::web::Data::new(token_manager))
            .configure(configure_routes),
    )
    .await;

    let req = test::TestRequest::get().uri("/health").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn test_unauthorized_access_without_token() {
    let token_manager = Arc::new(TokenManager::new());
    let app = test::init_service(
        App::new()
            .app_data(actix_web::web::Data::new(token_manager))
            .configure(configure_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/keys")
        .set_json(serde_json::json!({
            "key": "test",
            "value": "data"
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 401);
}

#[actix_web::test]
async fn test_valid_token_access() {
    let token_manager = Arc::new(TokenManager::new());
    let (token, _) = token_manager
        .create(
            "test-token".to_string(),
            vec![lsm_kv_store::api::auth::token::Permission::Write],
            None,
        )
        .unwrap();

    let app = test::init_service(
        App::new()
            .app_data(actix_web::web::Data::new(token_manager))
            .configure(configure_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/keys")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "key": "test",
            "value": "data"
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn test_token_management_flow() {
    let token_manager = Arc::new(TokenManager::new());
    let app = test::init_service(
        App::new()
            .app_data(actix_web::web::Data::new(token_manager.clone()))
            .configure(configure_routes),
    )
    .await;

    // Create token
    let create_req = test::TestRequest::post()
        .uri("/admin/tokens")
        .set_json(serde_json::json!({
            "name": "test-api",
            "permissions": ["Read", "Write"],
            "expiry_days": 30
        }))
        .to_request();

    let resp = test::call_service(&app, create_req).await;
    assert_eq!(resp.status(), 200);

    // List tokens
    let list_req = test::TestRequest::get().uri("/admin/tokens").to_request();
    let resp = test::call_service(&app, list_req).await;
    assert_eq!(resp.status(), 200);
}
```

### 2.6 Dependencies Update

**File**: `Cargo.toml` (add)

```toml
[dependencies]
# Existing dependencies...

# Authentication (Issue #31)
actix-web-httpauth = "0.8"
sha2 = "0.10"
uuid = { version = "1.6", features = ["v4", "serde"] }
rand = "0.8"
```

### 2.7 Documentation Updates

**File**: `README.md` (add section)

```markdown
## 🔐 Authentication

The API supports Bearer Token authentication to secure endpoints.

### Configuration

Set in `.env`:

```bash
API_AUTH_ENABLED=true
API_AUTH_SECRET=your-secret-key
API_TOKEN_EXPIRY_DAYS=30
```

### Creating a Token

```bash
curl -X POST http://localhost:8080/admin/tokens \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-app",
    "permissions": ["Read", "Write"],
    "expiry_days": 30
  }'
```

**Response**:
```json
{
  "token": "lsm_a1b2c3d4e5f6...",
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "my-app",
  "permissions": ["Read", "Write"]
}
```

### Using the Token

Include the `Authorization` header in all requests:

```bash
curl -X GET http://localhost:8080/keys/user:1 \
  -H "Authorization: Bearer lsm_a1b2c3d4e5f6..."
```

### Managing Tokens

**List all tokens**:
```bash
curl http://localhost:8080/admin/tokens
```

**Revoke a token**:
```bash
curl -X DELETE http://localhost:8080/admin/tokens/{token-id}
```

### Public Endpoints

The `/health` endpoint is always public and does not require authentication.

```bash
curl http://localhost:8080/health
```
```

**File**: `.env.example` (update)

```bash
# API Configuration
API_HOST=127.0.0.1
API_PORT=8080

# Authentication
API_AUTH_ENABLED=true
API_AUTH_SECRET=change-me-in-production
API_TOKEN_EXPIRY_DAYS=30

# Storage
DATA_DIR=./data
WAL_DIR=./data/wal

# Performance
MEMTABLE_SIZE_MB=64
BLOCK_SIZE=4096
BLOCK_CACHE_SIZE_MB=256
COMPACTION_THRESHOLD=4
```

---

## ✅ Implementation Checklist

### Phase 1: Concurrent Reads (#36)

- [ ] Update `SstableReader` struct with `Mutex<File>`
- [ ] Change `get(&mut self)` → `get(&self)`
- [ ] Change `scan(&mut self)` → `scan(&self)`
- [ ] Update `read_block()` to use file lock
- [ ] Add concurrency tests
- [ ] Update `LsmEngine` to use `&self` for reads
- [ ] Fix all existing tests (remove `&mut`)
- [ ] Run `cargo test --all-features`
- [ ] Run `cargo clippy`
- [ ] Optional: Add benchmark

### Phase 2: Bearer Token Auth (#31)

- [ ] Create `src/api/auth/` module structure
- [ ] Implement `token.rs` with tests
- [ ] Implement `error.rs`
- [ ] Implement `manager.rs` with tests
- [ ] Implement `middleware.rs`
- [ ] Implement `mod.rs`
- [ ] Update `.env` and `.env.example`
- [ ] Update `src/infra/config.rs` with `ApiConfig`
- [ ] Add token management handlers
- [ ] Update `src/api/server.rs` with auth middleware
- [ ] Create `tests/integration_auth.rs`
- [ ] Update `Cargo.toml` dependencies
- [ ] Update `README.md` with auth documentation
- [ ] Run `cargo test --all-features`
- [ ] Run `cargo clippy`
- [ ] Manual testing with `curl`

### Final Validation

- [ ] All tests pass (`cargo test --all-features`)
- [ ] No clippy warnings (`cargo clippy -- -D warnings`)
- [ ] Code compiles in release mode (`cargo build --release`)
- [ ] Documentation is clear and complete
- [ ] `.env.example` is updated
- [ ] Open PR to `develop` branch
- [ ] Link PR to issues #31 and #36

---

## 🚀 Expected Outcomes

### Issue #36 (Concurrent Reads)
- ✅ Multiple threads can read from the same SSTable simultaneously
- ✅ 3-10x throughput improvement in multi-threaded workloads
- ✅ No race conditions or deadlocks
- ✅ Minimal lock contention (file lock only during disk I/O)

### Issue #31 (Bearer Token Auth)
- ✅ All API endpoints protected except `/health`
- ✅ Token-based authentication with SHA-256 hashing
- ✅ Token management endpoints (create, list, revoke)
- ✅ Configurable via environment variables
- ✅ Backward compatible (can be disabled)
- ✅ Comprehensive test coverage

---

## 📊 Testing Strategy

### Unit Tests
- Token generation and validation
- Hash comparison (constant-time)
- Token expiry logic
- Permission checks
- Concurrent reader operations

### Integration Tests
- Unauthorized access returns 401
- Valid token grants access
- Public endpoints remain accessible
- Token management CRUD operations
- Concurrent reads from multiple threads

### Manual Testing
- Create token via API
- Use token to access protected endpoints
- Verify token expiry
- Test with invalid/missing tokens
- Stress test with concurrent clients

---

## 🔒 Security Considerations

1. **Token Storage**
   - Only hashed tokens stored (SHA-256)
   - Constant-time comparison to prevent timing attacks
   - Tokens never logged in plaintext

2. **Token Format**
   - Cryptographically secure random generation
   - 256 bits of entropy (64 hex chars)
   - Prefixed with `lsm_` for identification

3. **HTTPS Requirement**
   - Tokens transmitted only over HTTPS in production
   - Add warning if auth enabled without HTTPS

4. **Rate Limiting** (Future)
   - Per-token rate limits
   - Global rate limits
   - DDoS protection

---

## 📝 Notes

- This implementation prioritizes simplicity and correctness over advanced features
- Token storage is in-memory (lost on restart) - can be upgraded to persistent storage later
- JWT tokens deferred to future version if needed
- OAuth2 integration deferred to enterprise version

---

**Last Updated**: 2026-02-04  
**Status**: Ready for implementation  
**Estimated Completion**: 5-8 days
