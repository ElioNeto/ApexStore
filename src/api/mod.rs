mod config;

#[cfg(feature = "api")]
pub mod auth;

use actix_cors::Cors;
use actix_web::{
    delete, dev::ServiceRequest, get, post, web, App, Error, HttpResponse, HttpServer, Responder,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::core::engine::LsmEngine;
use crate::features::FeatureClient;

pub use config::{AuthConfig, ServerConfig};

#[cfg(feature = "api")]
use auth::{manager::TokenManager, middleware::extract_token, token::Permission, ApiToken};

pub struct AppState {
    pub engine: Arc<LsmEngine>,
    pub features: Arc<FeatureClient>,
    #[cfg(feature = "api")]
    pub token_manager: TokenManager,
    pub auth_enabled: bool,
}

#[derive(Deserialize)]
pub struct SetRequest {
    pub key: String,
    pub value: String,
}

#[derive(Deserialize)]
pub struct BatchSetRequest {
    pub records: Vec<SetRequest>,
}

#[derive(Deserialize)]
pub struct BatchDeleteRequest {
    pub keys: Vec<String>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub prefix: bool,
}

#[derive(Serialize)]
pub struct ApiResponse {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct SetFeatureRequest {
    pub enabled: bool,
    #[serde(default)]
    pub description: String,
}

#[derive(Serialize)]
pub struct FeatureResponse {
    pub name: String,
    pub enabled: bool,
    pub description: String,
}

// Admin endpoints
#[cfg(feature = "api")]
#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    pub permissions: Vec<Permission>,
    pub expires_in_days: Option<u32>,
}

#[cfg(feature = "api")]
#[derive(Serialize)]
pub struct TokenResponse {
    pub id: String,
    pub name: String,
    pub token: Option<String>,
    pub created_at: u128,
    pub expires_at: Option<u128>,
    pub permissions: Vec<Permission>,
}

// Public endpoint - no auth required
#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().json(ApiResponse {
        success: true,
        message: "ApexStore API is running".to_string(),
        data: None,
    })
}

#[get("/stats")]
async fn get_stats(data: web::Data<AppState>) -> impl Responder {
    let stats = data.engine.stats();
    HttpResponse::Ok().json(ApiResponse {
        success: true,
        message: "Stats retrieved".to_string(),
        data: Some(serde_json::json!({ "stats": stats })),
    })
}

#[get("/stats/all")]
async fn get_stats_all(data: web::Data<AppState>) -> impl Responder {
    match data.engine.stats_all() {
        Ok(stats) => HttpResponse::Ok().json(ApiResponse {
            success: true,
            message: "Stats retrieved".to_string(),
            data: Some(serde_json::to_value(stats).unwrap_or_default()),
        }),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

#[get("/keys/{key}")]
async fn get_key(path: web::Path<String>, data: web::Data<AppState>) -> impl Responder {
    let key = path.into_inner();

    match data.engine.get(&key) {
        Ok(Some(value)) => {
            let value_str = String::from_utf8_lossy(&value).to_string();
            HttpResponse::Ok().json(ApiResponse {
                success: true,
                message: "Key found".to_string(),
                data: Some(serde_json::json!({
                    "key": key,
                    "value": value_str
                })),
            })
        }
        Ok(None) => HttpResponse::NotFound().json(ApiResponse {
            success: false,
            message: format!("Key '{}' not found", key),
            data: None,
        }),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

#[post("/keys")]
async fn set_key(req: web::Json<SetRequest>, data: web::Data<AppState>) -> impl Responder {
    let value_bytes = req.value.as_bytes().to_vec();

    match data.engine.set(req.key.clone(), value_bytes) {
        Ok(_) => HttpResponse::Ok().json(ApiResponse {
            success: true,
            message: format!("Key '{}' set successfully", req.key),
            data: Some(serde_json::json!({ "key": req.key })),
        }),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

#[post("/keys/batch")]
async fn set_batch(req: web::Json<BatchSetRequest>, data: web::Data<AppState>) -> impl Responder {
    let records: Vec<(String, Vec<u8>)> = req
        .records
        .iter()
        .map(|r| (r.key.clone(), r.value.as_bytes().to_vec()))
        .collect();

    match data.engine.set_batch(records) {
        Ok(count) => HttpResponse::Ok().json(ApiResponse {
            success: true,
            message: format!("{} keys inserted successfully", count),
            data: Some(serde_json::json!({ "count": count })),
        }),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

#[delete("/keys/{key}")]
async fn delete_key(path: web::Path<String>, data: web::Data<AppState>) -> impl Responder {
    let key = path.into_inner();

    match data.engine.delete(key.clone()) {
        Ok(_) => HttpResponse::Ok().json(ApiResponse {
            success: true,
            message: format!("Key '{}' deleted successfully", key),
            data: None,
        }),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

#[get("/keys")]
async fn list_keys(data: web::Data<AppState>) -> impl Responder {
    match data.engine.keys() {
        Ok(keys) => {
            let filtered_keys: Vec<String> = keys
                .into_iter()
                .filter(|k: &String| !k.starts_with("feature:"))
                .collect();

            HttpResponse::Ok().json(ApiResponse {
                success: true,
                message: format!("{} keys found", filtered_keys.len()),
                data: Some(serde_json::json!({ "keys": filtered_keys })),
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

#[get("/keys/search")]
async fn search_keys(query: web::Query<SearchQuery>, data: web::Data<AppState>) -> impl Responder {
    let results = if query.prefix {
        data.engine.search_prefix(&query.q)
    } else {
        data.engine.search(&query.q)
    };

    match results {
        Ok(records) => {
            let records_json: Vec<serde_json::Value> = records
                .into_iter()
                .map(|(k, v): (String, Vec<u8>)| {
                    serde_json::json!({
                        "key": k,
                        "value": String::from_utf8_lossy(&v).to_string()
                    })
                })
                .collect();

            HttpResponse::Ok().json(ApiResponse {
                success: true,
                message: format!("{} keys found matching '{}'", records_json.len(), query.q),
                data: Some(serde_json::json!({ "records": records_json })),
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

#[get("/scan")]
async fn scan_all(data: web::Data<AppState>) -> impl Responder {
    match data.engine.scan() {
        Ok(records) => {
            let records_json: Vec<serde_json::Value> = records
                .into_iter()
                .filter(|(k, _): &(String, Vec<u8>)| !k.starts_with("feature:"))
                .map(|(k, v): (String, Vec<u8>)| {
                    serde_json::json!({
                        "key": k,
                        "value": String::from_utf8_lossy(&v).to_string()
                    })
                })
                .collect();

            HttpResponse::Ok().json(ApiResponse {
                success: true,
                message: format!("{} records found", records_json.len()),
                data: Some(serde_json::json!({ "records": records_json })),
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

#[get("/features")]
async fn list_features(data: web::Data<AppState>) -> impl Responder {
    match data.features.list_all() {
        Ok(features) => {
            let feature_list: Vec<FeatureResponse> = features
                .flags
                .iter()
                .map(|(name, flag)| FeatureResponse {
                    name: name.clone(),
                    enabled: flag.enabled,
                    description: flag.description.clone(),
                })
                .collect();

            HttpResponse::Ok().json(ApiResponse {
                success: true,
                message: format!("{} features found", feature_list.len()),
                data: Some(serde_json::json!({
                    "version": features.version,
                    "features": feature_list
                })),
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

#[post("/features/{name}")]
async fn set_feature(
    path: web::Path<String>,
    req: web::Json<SetFeatureRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    let name = path.into_inner();
    match data
        .features
        .set_flag(name.clone(), req.enabled, Some(req.description.clone()))
    {
        Ok(_) => HttpResponse::Ok().json(ApiResponse {
            success: true,
            message: format!("Feature '{}' updated", name),
            data: None,
        }),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

// Admin endpoints for token management
#[cfg(feature = "api")]
#[post("/admin/tokens")]
async fn create_token(
    req: web::Json<CreateTokenRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    let expires_at = req.expires_in_days.map(|days| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            + (days as u128 * 24 * 60 * 60 * 1_000_000_000)
    });

    match data
        .token_manager
        .create_token(req.name.clone(), expires_at, req.permissions.clone())
    {
        Ok((raw_token, token)) => HttpResponse::Ok().json(ApiResponse {
            success: true,
            message: "Token created successfully".to_string(),
            data: Some(serde_json::json!({
                "id": token.id,
                "name": token.name,
                "token": raw_token,
                "expires_at": token.expires_at,
                "permissions": token.permissions,
            })),
        }),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

#[cfg(feature = "api")]
#[get("/admin/tokens")]
async fn list_tokens(data: web::Data<AppState>) -> impl Responder {
    match data.token_manager.list_tokens() {
        Ok(tokens) => {
            let token_list: Vec<TokenResponse> = tokens
                .into_iter()
                .map(|t| TokenResponse {
                    id: t.id,
                    name: t.name,
                    token: None, // Never expose raw token
                    created_at: t.created_at,
                    expires_at: t.expires_at,
                    permissions: t.permissions,
                })
                .collect();

            HttpResponse::Ok().json(ApiResponse {
                success: true,
                message: format!("{} tokens found", token_list.len()),
                data: Some(serde_json::json!({ "tokens": token_list })),
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

#[cfg(feature = "api")]
#[delete("/admin/tokens/{id}")]
async fn delete_token(path: web::Path<String>, data: web::Data<AppState>) -> impl Responder {
    let id = path.into_inner();

    match data.token_manager.delete_token(&id) {
        Ok(_) => HttpResponse::Ok().json(ApiResponse {
            success: true,
            message: "Token deleted successfully".to_string(),
            data: None,
        }),
        Err(e) => HttpResponse::NotFound().json(ApiResponse {
            success: false,
            message: format!("Error: {}", e),
            data: None,
        }),
    }
}

// Custom auth validator
#[cfg(feature = "api")]
async fn auth_validator(
    req: ServiceRequest,
    credentials: actix_web_httpauth::extractors::bearer::BearerAuth,
) -> Result<ServiceRequest, (Error, ServiceRequest)> {
    let data = req.app_data::<web::Data<AppState>>().unwrap();
    
    if !data.auth_enabled {
        return Ok(req);
    }

    auth::middleware::bearer_validator(
        req,
        data.token_manager.clone(),
        Some(credentials.token().to_string()),
    )
    .await
}

pub async fn start_server(
    engine: LsmEngine,
    server_config: ServerConfig,
) -> std::io::Result<()> {
    let engine = Arc::new(engine);
    let features = Arc::new(FeatureClient::new(
        Arc::clone(&engine),
        Duration::from_secs(server_config.feature_cache_ttl_secs),
    ));

    #[cfg(feature = "api")]
    let token_manager = TokenManager::new();

    #[cfg(feature = "api")]
    let auth_enabled = server_config.auth.enabled;

    server_config.print_info();
    println!("🚀 Starting server at {}:{}\n", server_config.host, server_config.port);

    let max_json = server_config.max_json_payload_size;
    let max_raw = server_config.max_raw_payload_size;
    let host = server_config.host.clone();
    let port = server_config.port;

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();

        #[cfg(feature = "api")]
        let auth_middleware =
            actix_web_httpauth::middleware::HttpAuthentication::bearer(auth_validator);

        let mut app = App::new()
            .wrap(cors)
            .app_data(web::Data::new(AppState {
                engine: Arc::clone(&engine),
                features: Arc::clone(&features),
                #[cfg(feature = "api")]
                token_manager: token_manager.clone(),
                #[cfg(feature = "api")]
                auth_enabled,
                #[cfg(not(feature = "api"))]
                auth_enabled: false,
            }))
            .app_data(web::JsonConfig::default().limit(max_json))
            .app_data(web::PayloadConfig::default().limit(max_raw))
            // Public endpoints (no auth)
            .service(health);

        // Protected endpoints (with conditional auth)
        #[cfg(feature = "api")]
        {
            app = app
                .service(
                    web::scope("")
                        .wrap(auth_middleware.clone())
                        .service(get_stats)
                        .service(get_stats_all)
                        .service(get_key)
                        .service(set_key)
                        .service(set_batch)
                        .service(delete_key)
                        .service(list_keys)
                        .service(search_keys)
                        .service(scan_all)
                        .service(list_features)
                        .service(set_feature),
                )
                .service(
                    web::scope("/admin")
                        .wrap(auth_middleware)
                        .service(create_token)
                        .service(list_tokens)
                        .service(delete_token),
                );
        }

        #[cfg(not(feature = "api"))]
        {
            app = app
                .service(get_stats)
                .service(get_stats_all)
                .service(get_key)
                .service(set_key)
                .service(set_batch)
                .service(delete_key)
                .service(list_keys)
                .service(search_keys)
                .service(scan_all)
                .service(list_features)
                .service(set_feature);
        }

        app
    })
    .bind((host.as_str(), port))?
    .run()
    .await
}
