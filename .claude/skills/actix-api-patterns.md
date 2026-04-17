# Skill: Padrões Actix-Web — ApexStore API

Use esta skill ao criar ou modificar handlers REST em `src/api/`.

## Estado compartilhado

O Engine é injetado via `web::Data`. Sempre usar `Arc<RwLock<LsmEngine>>`:

```rust
// Registro (src/bin/server.rs)
let engine = Arc::new(RwLock::new(LsmEngine::new(&config)?));

HttpServer::new(move || {
    App::new()
        .app_data(web::Data::new(engine.clone()))
        .service(web::scope("/keys")
            .route("", web::post().to(put_key))
            .route("/{key}", web::get().to(get_key))
        )
        .route("/stats/all", web::get().to(get_stats))
})
```

## Template de handler

```rust
use actix_web::{web, HttpResponse, Responder};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::instrument;
use crate::core::engine::LsmEngine;
use crate::infra::error::ApexError;

#[derive(Deserialize)]
pub struct MyRequest {
    pub key: String,
    pub value: String,
}

#[derive(Serialize)]
pub struct MyResponse {
    pub result: String,
}

#[instrument(skip(engine))]
pub async fn my_handler(
    engine: web::Data<Arc<RwLock<LsmEngine>>>,
    body: web::Json<MyRequest>,
) -> impl Responder {
    let guard = engine.read();
    match guard.some_operation(&body.key) {
        Ok(result) => HttpResponse::Ok().json(MyResponse { result }),
        Err(ApexError::KeyNotFound) => {
            HttpResponse::NotFound().json(serde_json::json!({ "error": "Key not found" }))
        }
        Err(e) => {
            tracing::error!(error = %e, "handler error");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": e.to_string() }))
        }
    }
}
```

## Mapeamento de erros HTTP

| `ApexError` | HTTP Status |
|---|---|
| `KeyNotFound` | 404 Not Found |
| `InvalidKey` | 400 Bad Request |
| `MemTableFull` | 503 Service Unavailable |
| `IoError` | 500 Internal Server Error |
| `CodecError` | 500 Internal Server Error |
| Qualquer outro | 500 Internal Server Error |

## Validação de request

```rust
// Validação manual (sem lib externa)
pub async fn put_key(
    engine: web::Data<Arc<RwLock<LsmEngine>>>,
    body: web::Json<KeyValueRequest>,
) -> impl Responder {
    if body.key.is_empty() {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "error": "Key cannot be empty" }));
    }
    if body.key.len() > 1024 {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "error": "Key too long (max 1024 bytes)" }));
    }
    // ...
}
```

## CORS

Configurado globalmente em `src/bin/server.rs`. Não configurar por-handler. Para adicionar origens:
```rust
Cors::default()
    .allowed_origin("http://localhost:4200")  // Angular dev
    .allowed_origin(&config.cors_origin)       // produção via env
    .allowed_methods(vec!["GET", "POST", "DELETE"])
    .allowed_headers(vec![header::CONTENT_TYPE, header::AUTHORIZATION])
```

## Path params e query params

```rust
// Path: GET /keys/{key}
pub async fn get_key(
    path: web::Path<String>,
    engine: web::Data<Arc<RwLock<LsmEngine>>>,
) -> impl Responder {
    let key = path.into_inner();
    // ...
}

// Query: GET /keys?prefix=user:
#[derive(Deserialize)]
pub struct SearchQuery {
    pub prefix: Option<String>,
    pub limit: Option<usize>,
}

pub async fn search_keys(
    query: web::Query<SearchQuery>,
    engine: web::Data<Arc<RwLock<LsmEngine>>>,
) -> impl Responder {
    let prefix = query.prefix.as_deref().unwrap_or("");
    // ...
}
```

## Autenticação Bearer

Já implementado via `actix-web-httpauth`. Para proteger novas rotas:
```rust
use actix_web_httpauth::middleware::HttpAuthentication;

web::scope("/admin")
    .wrap(HttpAuthentication::bearer(validator))
    .route("/flush", web::post().to(force_flush))
```
