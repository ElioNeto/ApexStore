//! Real-time change notifications via WebSocket and Server-Sent Events.
//!
//! Provides:
//! - `GET /ws/events` — WebSocket upgrade that streams CDC events
//! - `GET /events` — SSE endpoint for HTTP-only clients
//!
//! Both support optional filtering by key prefix and event type.

use actix_web::{get, web, HttpRequest, HttpResponse};
use actix_ws::Message;
use futures::StreamExt;
use serde_json::json;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::broadcast;

use super::auth::{require_permission, Permission};
use crate::infra::cdc::CdcEvent;
use crate::infra::events::{EventBus, EventQuery};

// ── WebSocket handler ───────────────────────────────────────────────────────

/// Handler for `GET /ws/events` — WebSocket upgrade for real-time CDC events.
#[get("/ws/events")]
pub async fn ws_events(
    req: HttpRequest,
    body: web::Payload,
    event_bus: web::Data<EventBus>,
    query: web::Query<EventQuery>,
) -> Result<HttpResponse, actix_web::Error> {
    // Permission check
    if let Err(e) = require_permission(&req, Permission::Read) {
        return Ok(e);
    }

    if !event_bus.is_enabled() {
        return Ok(HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "error": "event bus is not enabled" })));
    }

    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, body)?;

    let mut rx = event_bus.subscribe();
    let filter = query.into_inner();

    // Spawn a task to forward CDC events to the WebSocket
    actix_web::rt::spawn(async move {
        // Send an initial "connected" message
        let connect_msg = serde_json::json!({
            "type": "connected",
            "message": "CDC event stream connected"
        });
        if session.text(connect_msg.to_string()).await.is_err() {
            return;
        }

        loop {
            tokio::select! {
                // Incoming WebSocket messages (keep-alive pings)
                msg = msg_stream.next() => {
                    match msg {
                        Some(Ok(Message::Ping(bytes))) => {
                            let _ = session.pong(&bytes).await;
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => {}
                    }
                }
                // CDC events from the event bus
                event = rx.recv() => {
                    match event {
                        Ok(event) => {
                            if !filter.matches(&event) {
                                continue;
                            }
                            let event_json = serde_json::to_string(&event).unwrap_or_default();
                            if session.text(event_json).await.is_err() {
                                break; // Client disconnected
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(
                                target: "apexstore::ws",
                                "WebSocket client lagged by {} events",
                                n
                            );
                            let lag_msg = serde_json::json!({
                                "type": "lag",
                                "skipped": n
                            });
                            let _ = session.text(lag_msg.to_string()).await;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    });

    Ok(response)
}

// ── SSE handler ─────────────────────────────────────────────────────────────

// ── SSE Stream ──────────────────────────────────────────────────────────────

/// A stream that yields SSE-formatted CDC events.
struct SseEventStream {
    rx: broadcast::Receiver<CdcEvent>,
    filter: EventQuery,
    initial_sent: bool,
}

impl futures::Stream for SseEventStream {
    type Item = Result<actix_web::web::Bytes, actix_web::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Send initial connected event first
        if !self.initial_sent {
            self.initial_sent = true;
            let connect_data = serde_json::json!({
                "type": "connected",
                "message": "CDC event stream connected"
            });
            return Poll::Ready(Some(Ok(actix_web::web::Bytes::from(format!(
                "data: {}\n\n",
                connect_data
            )))));
        }

        loop {
            match self.rx.try_recv() {
                Ok(event) => {
                    if !self.filter.matches(&event) {
                        continue;
                    }
                    let event_json = serde_json::to_string(&event).unwrap_or_default();
                    let sse_data = format!("data: {}\n\n", event_json);
                    return Poll::Ready(Some(Ok(actix_web::web::Bytes::from(sse_data))));
                }
                Err(broadcast::error::TryRecvError::Empty) => {
                    // No event available — register waker and return pending
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Err(broadcast::error::TryRecvError::Closed) => {
                    return Poll::Ready(None);
                }
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    let lag_msg = serde_json::json!({
                        "type": "lag",
                        "skipped": n
                    });
                    let sse_data = format!("data: {}\n\n", lag_msg);
                    return Poll::Ready(Some(Ok(actix_web::web::Bytes::from(sse_data))));
                }
            }
        }
    }
}

/// Handler for `GET /events` — Server-Sent Events for real-time CDC updates.
#[get("/events")]
pub async fn sse_events(
    req: HttpRequest,
    event_bus: web::Data<EventBus>,
    query: web::Query<EventQuery>,
) -> HttpResponse {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }

    if !event_bus.is_enabled() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "error": "event bus is not enabled" }));
    }

    let rx = event_bus.subscribe();
    let filter = query.into_inner();

    let stream = SseEventStream {
        rx,
        filter,
        initial_sent: false,
    };

    HttpResponse::Ok()
        .content_type("text/event-stream")
        .streaming(stream)
}
