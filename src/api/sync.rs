//! Real-time sync via WebSocket with CRDT-based conflict resolution.
//!
//! Provides a WebSocket endpoint at `/ws/sync` for bidirectional sync.
//! Uses the existing CRDT engine for last-writer-wins conflict resolution.

use actix_web::error::InternalError;
use actix_web::{get, web, HttpRequest, HttpResponse};
use actix_ws::Message;

use super::auth::middleware::extract_token;
use super::auth::{require_permission, Permission, TokenManager};
use super::config::{AccessControlEnabled, AuthEnabled};
use crate::infra::access_control::{AccessController, Operation};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A change entry in the sync protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncChange {
    pub key: String,
    pub value: String,
    pub timestamp: u64,
    pub device_id: String,
}

/// Message sent from client to server.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "sync_push")]
    SyncPush {
        changes: Vec<SyncChange>,
        last_ack: u64,
    },
    #[serde(rename = "sync_ack")]
    SyncAck { ack_timestamp: u64 },
    #[serde(rename = "subscribe")]
    Subscribe { note_path: String },
}

/// Message sent from server to client.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "sync_ack")]
    SyncAck {
        ack_timestamp: u64,
        changes: Vec<SyncChange>,
    },
    #[serde(rename = "sync_push")]
    SyncPush { changes: Vec<SyncChange> },
    #[serde(rename = "error")]
    Error { message: String },
}

/// Manages connected WebSocket clients and CRDT state.
pub struct SyncManager {
    /// Connected clients (note_path -> broadcast channel).
    clients: Mutex<Vec<tokio::sync::mpsc::UnboundedSender<String>>>,
    /// CRDT engine for conflict resolution.
    crdt: Mutex<crate::infra::crdt::CrdtEngine>,
}

impl SyncManager {
    pub fn new() -> Self {
        Self {
            clients: Mutex::new(Vec::new()),
            crdt: Mutex::new(crate::infra::crdt::CrdtEngine::new()),
        }
    }
}

impl Default for SyncManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncManager {
    /// Register a new client connection.
    pub fn register_client(&self, tx: tokio::sync::mpsc::UnboundedSender<String>) {
        let mut clients = self.clients.lock();
        clients.push(tx);
    }

    /// Remove a disconnected client.
    pub fn remove_client(&self, tx: &tokio::sync::mpsc::UnboundedSender<String>) {
        let mut clients = self.clients.lock();
        clients.retain(|c| !c.same_channel(tx));
    }

    /// Broadcast a change to all connected clients.
    pub fn broadcast(&self, change: &SyncChange) {
        let msg = match serde_json::to_string(&ServerMessage::SyncPush {
            changes: vec![change.clone()],
        }) {
            Ok(message) => message,
            Err(error) => {
                tracing::error!(%error, "failed to serialize sync broadcast");
                return;
            }
        };
        let mut clients = self.clients.lock();
        clients.retain(|c| c.send(msg.clone()).is_ok());
    }

    /// Persist a winning change before updating CRDT state or broadcasting it.
    pub fn apply_change(
        &self,
        change: &SyncChange,
        engine: &crate::LsmEngine,
        cf: &str,
    ) -> crate::Result<()> {
        if change.key.starts_with("__") {
            return Err(crate::LsmError::InvalidArgument("reserved sync key".into()));
        }
        let mut crdt = self.crdt.lock();
        if crdt
            .get_state(change.key.as_bytes())
            .is_some_and(|(_, timestamp)| timestamp >= change.timestamp)
        {
            return Ok(());
        }
        engine.put_cf(
            cf,
            change.key.as_bytes().to_vec(),
            change.value.as_bytes().to_vec(),
        )?;
        crdt.merge(
            change.key.as_bytes().to_vec(),
            change.value.as_bytes().to_vec(),
            change.timestamp,
        );
        drop(crdt);
        self.broadcast(change);
        Ok(())
    }

    /// Get pending changes since a given timestamp.
    pub fn get_changes_since(&self, since: u64) -> Vec<SyncChange> {
        let crdt = self.crdt.lock();
        crdt.get_all_entries()
            .into_iter()
            .filter(|e| e.timestamp > since)
            .map(|e| SyncChange {
                key: String::from_utf8_lossy(&e.key).to_string(),
                value: String::from_utf8_lossy(&e.value).to_string(),
                timestamp: e.timestamp,
                device_id: "server".to_string(),
            })
            .collect()
    }
}

/// Refresh permission and expiry for every message on a long-lived connection.
struct SyncAuthorization {
    disabled: bool,
    token_id: Option<String>,
    manager: Option<web::Data<TokenManager>>,
    access_enabled: bool,
    controller: Option<web::Data<AccessController>>,
}

impl SyncAuthorization {
    fn from_request(req: &HttpRequest) -> Self {
        Self {
            disabled: req
                .app_data::<web::Data<AuthEnabled>>()
                .is_some_and(|flag| !flag.0),
            token_id: extract_token(req).map(|token| token.id),
            manager: req.app_data::<web::Data<TokenManager>>().cloned(),
            access_enabled: req
                .app_data::<web::Data<AccessControlEnabled>>()
                .is_some_and(|flag| flag.0),
            controller: req.app_data::<web::Data<AccessController>>().cloned(),
        }
    }

    fn permits(&self, permission: Permission, key: &str) -> bool {
        if key.starts_with("__") {
            return false;
        }
        let mut context = HashMap::new();
        if !self.disabled {
            let Some(token) = self
                .manager
                .as_ref()
                .zip(self.token_id.as_ref())
                .and_then(|(manager, id)| manager.get_token(id).ok())
            else {
                return false;
            };
            if !token.has_permission(permission) || token.is_expired().unwrap_or(true) {
                return false;
            }
            context.insert("principal".to_string(), token.name);
            context.insert(
                "permissions".to_string(),
                token
                    .permissions
                    .iter()
                    .map(|permission| format!("{permission:?}"))
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        !self.access_enabled
            || self.controller.as_ref().is_some_and(|controller| {
                let operation = match permission {
                    Permission::Read => Operation::Read,
                    Permission::Write => Operation::Write,
                    Permission::Delete => Operation::Delete,
                    Permission::Admin => Operation::Admin,
                };
                controller.check_permission(&operation, key.as_bytes(), &context)
            })
    }
}

async fn send_message(session: &mut actix_ws::Session, message: &ServerMessage) -> bool {
    match serde_json::to_string(message) {
        Ok(json) => session.text(json).await.is_ok(),
        Err(error) => {
            tracing::error!(%error, "failed to serialize sync response");
            false
        }
    }
}

/// WebSocket handler at `/ws/sync`.
#[get("/ws/sync")]
pub async fn sync_handler(
    req: HttpRequest,
    body: web::Payload,
    sync_manager: web::Data<SyncManager>,
    engine: web::Data<crate::LsmEngine>,
) -> Result<HttpResponse, actix_web::Error> {
    // This bidirectional endpoint accepts writes; read-only tokens cannot upgrade.
    if let Err(resp) = require_permission(&req, Permission::Write) {
        return Err(InternalError::from_response("WebSocket auth failed", *resp).into());
    }

    let authorization = SyncAuthorization::from_request(&req);
    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, body)?;

    // Create a channel for sending messages to this client
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    sync_manager.register_client(tx.clone());

    // Clone Data references for the task
    let sync_mgr = sync_manager.clone();
    let engine_data = engine.clone();

    // Single task handling both inbound and outbound messages
    actix_rt::spawn(async move {
        loop {
            tokio::select! {
                // Outbound: forward messages from channel to WebSocket
                Some(msg) = rx.recv() => {
                    if let Ok(ServerMessage::SyncPush { changes }) = serde_json::from_str(&msg) {
                        let changes: Vec<_> = changes.into_iter()
                            .filter(|change| authorization.permits(Permission::Read, &change.key))
                            .collect();
                        if !changes.is_empty() && !send_message(&mut session, &ServerMessage::SyncPush { changes }).await {
                            break;
                        }
                    }
                }
                // Inbound: handle incoming WebSocket messages
                incoming = msg_stream.recv() => {
                    let Some(Ok(msg)) = incoming else {
                        break;
                    };
                    match msg {
                        Message::Text(text) => {
                            if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                                match client_msg {
                                    ClientMessage::SyncPush { changes, last_ack } => {
                                        // Validate the entire batch before its first mutation.
                                        if changes.iter().any(|change| !authorization.permits(Permission::Write, &change.key)) {
                                            if !send_message(&mut session, &ServerMessage::Error { message: "sync write denied".into() }).await {
                                                break;
                                            }
                                            continue;
                                        }
                                        let mut failure = None;
                                        for change in &changes {
                                            if !authorization.permits(Permission::Write, &change.key) {
                                                failure = Some("sync write denied");
                                                break;
                                            }
                                            if let Err(error) = sync_mgr.apply_change(change, &engine_data, "default") {
                                                tracing::error!(%error, "sync storage write failed");
                                                failure = Some("sync storage write failed");
                                                break;
                                            }
                                        }
                                        if let Some(message) = failure {
                                            if !send_message(&mut session, &ServerMessage::Error { message: message.into() }).await {
                                                break;
                                            }
                                            continue;
                                        }
                                        let pending = sync_mgr.get_changes_since(last_ack).into_iter()
                                            .filter(|change| authorization.permits(Permission::Read, &change.key)).collect();
                                        let ack = ServerMessage::SyncAck {
                                            ack_timestamp: changes.iter().map(|change| change.timestamp).max().unwrap_or(last_ack),
                                            changes: pending,
                                        };
                                        if !send_message(&mut session, &ack).await {
                                            break;
                                        }
                                    }
                                    ClientMessage::SyncAck { ack_timestamp: _ } => {}
                                    ClientMessage::Subscribe { note_path: _ } => {}
                                }
                            }
                        }
                        Message::Ping(bytes) => {
                            let _ = session.pong(&bytes).await;
                        }
                        Message::Close(_) => {
                            break;
                        }
                        _ => {}
                    }
                }
                else => {
                    break;
                }
            }
        }
        sync_mgr.remove_client(&tx);
    });

    Ok(response)
}

/// Get all current CRDT entries (for REST API).
pub fn get_all_entries(sync_manager: &SyncManager) -> Vec<crate::infra::crdt::CrdtEntry> {
    sync_manager.crdt.lock().get_all_entries()
}
