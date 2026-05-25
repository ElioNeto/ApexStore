//! Notes CRUD and graph REST API endpoints.
//!
//! Provides the following endpoints:
//!
//! - `GET    /notes`                          — List all notes
//! - `GET    /notes/{path}`                   — Get a note
//! - `PUT    /notes/{path}`                   — Create or update a note
//! - `DELETE /notes/{path}`                   — Delete a note
//! - `POST   /notes/{path}/rename`            — Rename a note
//! - `GET    /notes/{path}/backlinks`         — List backlinks
//! - `GET    /notes/{path}/links`             — List forward links
//! - `GET    /notes/{path}/graph`             — Graph view data
//! - `GET    /tags`                           — List all tags
//! - `GET    /tags/{tag}/notes`               — List notes by tag

use crate::infra::error::LsmError;
use crate::notes::{GraphConfig, GraphDepth, NotesEngine};
use actix_web::{delete, get, post, put, web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use serde_json::json;

/// Query parameters for `GET /notes`
#[derive(Deserialize)]
pub struct ListNotesQuery {
    prefix: Option<String>,
}

/// Request body for `PUT /notes/{path}`
#[derive(Deserialize)]
pub struct PutNoteBody {
    content: String,
}

/// Query parameters for `GET /notes/{path}/graph`
#[derive(Deserialize)]
pub struct GraphQuery {
    depth: Option<usize>,
    max_nodes: Option<usize>,
    tag_filter: Option<String>,
}

/// Query parameters for `GET /tags/{tag}/notes`
#[derive(Deserialize)]
pub struct TagNotesQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

/// Request body for `POST /notes/{path}/rename`
#[derive(Deserialize)]
pub struct RenameBody {
    new_path: String,
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// `GET /notes` — list all notes with optional prefix filter.
#[get("")]
async fn list_notes(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    query: web::Query<ListNotesQuery>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Read) {
        return e;
    }
    match engine.list_notes(query.prefix.as_deref()) {
        Ok(notes) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "notes": notes })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to list notes: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `GET /notes/{path}` — get a single note.
#[get("/{path}")]
async fn get_note(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Read) {
        return e;
    }
    let note_path = path.into_inner();
    match engine.get_note(&note_path) {
        Ok(Some(content)) => {
            // Parse to return rich data
            let parsed = crate::notes::parse_note(&content);
            let backlinks = engine.get_backlinks(&note_path).unwrap_or_default();
            let forward = engine.get_forward_links(&note_path).unwrap_or_default();

            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({
                    "path": note_path,
                    "content": content,
                    "frontmatter": parsed.frontmatter,
                    "links": forward,
                    "backlinks": backlinks,
                    "tags": parsed.inline_tags,
                }))
        }
        Ok(None) => HttpResponse::NotFound()
            .content_type("application/json")
            .json(json!({ "error": "note not found" })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to get note: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `PUT /notes/{path}` — create or update a note.
#[put("/{path}")]
async fn put_note(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
    body: web::Json<PutNoteBody>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Write) {
        return e;
    }
    let note_path = path.into_inner();
    match engine.put_note(&note_path, &body.content) {
        Ok(_) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "status": "ok", "path": note_path })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to put note: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `DELETE /notes/{path}` — delete a note.
#[delete("/{path}")]
async fn delete_note(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Delete) {
        return e;
    }
    let note_path = path.into_inner();
    match engine.delete_note(&note_path) {
        Ok(_) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "status": "ok" })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to delete note: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `POST /notes/{path}/rename` — rename a note.
#[post("/{path}/rename")]
async fn rename_note(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
    body: web::Json<RenameBody>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Write) {
        return e;
    }
    let old_path = path.into_inner();
    match engine.rename_note(&old_path, &body.new_path) {
        Ok(_) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "status": "ok", "old_path": old_path, "new_path": body.new_path })),
        Err(LsmError::InvalidArgument(msg)) => HttpResponse::NotFound()
            .content_type("application/json")
            .json(json!({ "error": msg })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to rename note: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `GET /notes/{path}/backlinks` — list notes linking TO this note.
#[get("/{path}/backlinks")]
async fn get_backlinks(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Read) {
        return e;
    }
    let note_path = path.into_inner();
    match engine.get_backlinks(&note_path) {
        Ok(links) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "backlinks": links })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to get backlinks: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `GET /notes/{path}/links` — list notes linked FROM this note.
#[get("/{path}/links")]
async fn get_forward_links(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Read) {
        return e;
    }
    let note_path = path.into_inner();
    match engine.get_forward_links(&note_path) {
        Ok(links) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "links": links })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to get forward links: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `GET /notes/{path}/graph` — graph view data.
#[get("/{path}/graph")]
async fn get_graph(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
    query: web::Query<GraphQuery>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Read) {
        return e;
    }
    let note_path = path.into_inner();

    // Build graph config from query params
    let depth = match query.depth.unwrap_or(1) {
        0 => GraphDepth::Direct,
        1 => GraphDepth::Direct,
        2 => GraphDepth::Extended,
        _ => GraphDepth::Deep,
    };
    let max_nodes = query.max_nodes.unwrap_or(500).min(500);
    let config = GraphConfig {
        depth,
        max_nodes,
        tag_filter: query.tag_filter.clone(),
        include_tags: true,
        include_isolated: false,
    };

    match engine.build_graph(&note_path, &config) {
        Ok(graph_data) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({
                "root": graph_data.root,
                "nodes": graph_data.nodes,
                "edges": graph_data.edges,
            })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to build graph: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `GET /tags` — list all tags.
#[get("/tags")]
async fn list_tags(req: HttpRequest, engine: web::Data<NotesEngine>) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Read) {
        return e;
    }
    match engine.list_tags() {
        Ok(tags) => {
            let tags_json: Vec<serde_json::Value> = tags
                .into_iter()
                .map(|(name, count)| json!({ "tag": name, "count": count }))
                .collect();
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "tags": tags_json }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to list tags: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `GET /tags/{tag}/notes` — list notes with a specific tag.
#[get("/tags/{tag}/notes")]
async fn get_notes_by_tag(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
    query: web::Query<TagNotesQuery>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Read) {
        return e;
    }
    let tag = path.into_inner();
    match engine.search_by_tag(&tag, query.cursor.as_deref(), query.limit.unwrap_or(50)) {
        Ok((notes, next_cursor)) => {
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({
                    "tag": tag,
                    "notes": notes,
                    "cursor": next_cursor,
                }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to get notes by tag: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

// ── Route configuration ─────────────────────────────────────────────────────

/// Register all notes API routes under `/notes` and `/tags`.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/notes")
            .service(list_notes)
            .service(get_note)
            .service(put_note)
            .service(delete_note)
            .service(rename_note)
            .service(get_backlinks)
            .service(get_forward_links)
            .service(get_graph),
    )
    .service(list_tags)
    .service(get_notes_by_tag);
}
