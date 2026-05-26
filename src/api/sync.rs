//! Real-time sync via WebSocket with CRDT-based conflict resolution.
//!
//! Provides a WebSocket endpoint at `/ws/sync` for bidirectional sync.
//! Uses the existing CRDT engine for last-writer-wins conflict resolution.

use actix_web::error::InternalError;
use actix_web::{get, web, HttpRequest, HttpResponse};
use actix_ws::Message;

use super::auth::{require_permission, Permission};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

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
#[derive(Debug, Serialize)]
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
        let mut clients = self.clients.lock().unwrap();
        clients.push(tx);
    }

    /// Remove a disconnected client.
    pub fn remove_client(&self, tx: &tokio::sync::mpsc::UnboundedSender<String>) {
        let mut clients = self.clients.lock().unwrap();
        clients.retain(|c| !c.same_channel(tx));
    }

    /// Broadcast a change to all connected clients.
    pub fn broadcast(&self, change: &SyncChange) {
        let msg = serde_json::to_string(&ServerMessage::SyncPush {
            changes: vec![change.clone()],
        })
        .unwrap_or_default();
        let mut clients = self.clients.lock().unwrap();
        clients.retain(|c| c.send(msg.clone()).is_ok());
    }

    /// Merge a change into the CRDT engine and persist to storage.
    pub fn apply_change(&self, change: &SyncChange, engine: &crate::LsmEngine, cf: &str) {
        let mut crdt = self.crdt.lock().unwrap();
        crdt.merge(
            change.key.as_bytes().to_vec(),
            change.value.as_bytes().to_vec(),
            change.timestamp,
        );

        // Persist to LSM engine
        let _ = engine.put_cf(
            cf,
            change.key.as_bytes().to_vec(),
            change.value.as_bytes().to_vec(),
        );

        // Broadcast to other clients
        self.broadcast(change);
    }

    /// Get pending changes since a given timestamp.
    pub fn get_changes_since(&self, since: u64) -> Vec<SyncChange> {
        let crdt = self.crdt.lock().unwrap();
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

/// WebSocket handler at `/ws/sync`.
#[get("/ws/sync")]
pub async fn sync_handler(
    req: HttpRequest,
    body: web::Payload,
    sync_manager: web::Data<SyncManager>,
    engine: web::Data<crate::LsmEngine>,
) -> Result<HttpResponse, actix_web::Error> {
    // Authentication check — require at least Read permission
    if let Err(resp) = require_permission(&req, Permission::Read) {
        return Err(InternalError::from_response("WebSocket auth failed", resp).into());
    }

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
                    if session.text(msg).await.is_err() {
                        break;
                    }
                }
                // Inbound: handle incoming WebSocket messages
                Some(Ok(msg)) = msg_stream.recv() => {
                    match msg {
                        Message::Text(text) => {
                            if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                                match client_msg {
                                    ClientMessage::SyncPush { changes, last_ack } => {
                                        for change in &changes {
                                            sync_mgr.apply_change(change, &engine_data, "default");
                                        }
                                        let pending = sync_mgr.get_changes_since(last_ack);
                                        let ack = ServerMessage::SyncAck {
                                            ack_timestamp: changes
                                                .iter()
                                                .map(|c| c.timestamp)
                                                .max()
                                                .unwrap_or(last_ack),
                                            changes: pending,
                                        };
                                        if let Ok(json) = serde_json::to_string(&ack) {
                                            let _ = tx.send(json);
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
    sync_manager.crdt.lock().unwrap().get_all_entries()
}
