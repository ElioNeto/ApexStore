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
use std::sync::Mutex;

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

/// Query parameters for `POST /notes/{path}/restore`
#[derive(Deserialize)]
pub struct RestoreQuery {
    timestamp: u128,
}

/// Query parameters for `GET /search`
#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    limit: Option<usize>,
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
    match engine.put_note_with_version(&note_path, &body.content) {
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

/// `GET /notes/{path}/history` — list version history for a note.
#[get("/{path}/history")]
async fn get_version_history(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Read) {
        return e;
    }
    let note_path = path.into_inner();
    match engine.get_version_history(&note_path) {
        Ok(timestamps) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "path": note_path, "versions": timestamps })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to get version history: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `GET /notes/{path}/history/{timestamp}` — get note at a specific version.
#[get("/{path}/history/{timestamp}")]
async fn get_note_at_version(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<(String, u128)>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Read) {
        return e;
    }
    let (note_path, timestamp) = path.into_inner();
    match engine.get_note_at_version(&note_path, timestamp) {
        Ok(Some(content)) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "path": note_path, "timestamp": timestamp, "content": content })),
        Ok(None) => HttpResponse::NotFound()
            .content_type("application/json")
            .json(json!({ "error": "version not found" })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to get note at version: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `DELETE /notes/{path}/history/{timestamp}` — remove a specific version.
#[delete("/{path}/history/{timestamp}")]
async fn delete_version(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<(String, u128)>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Delete) {
        return e;
    }
    let (note_path, timestamp) = path.into_inner();
    match engine.remove_version(&note_path, timestamp) {
        Ok(true) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "status": "ok" })),
        Ok(false) => HttpResponse::NotFound()
            .content_type("application/json")
            .json(json!({ "error": "version not found" })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to remove version: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `POST /notes/{path}/restore?timestamp=...` — restore note from a version.
#[post("/{path}/restore")]
async fn restore_version(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
    query: web::Query<RestoreQuery>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Write) {
        return e;
    }
    let note_path = path.into_inner();
    match engine.restore_version(&note_path, query.timestamp) {
        Ok(true) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "status": "ok", "path": note_path, "restored_from": query.timestamp })),
        Ok(false) => HttpResponse::NotFound()
            .content_type("application/json")
            .json(json!({ "error": "version not found" })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to restore version: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `POST /notes/{path}/snapshot` — create a manual TimeTravel snapshot.
#[post("/{path}/snapshot")]
async fn create_snapshot(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
    time_travel: web::Data<Mutex<crate::infra::time_travel::TimeTravelEngine>>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Admin) {
        return e;
    }
    let note_path = path.into_inner();
    let content = match engine.get_note(&note_path) {
        Ok(Some(c)) => c,
        Ok(None) => {
            return HttpResponse::NotFound()
                .content_type("application/json")
                .json(json!({ "error": "note not found" }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to get note: {:?}", e);
            return HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }));
        }
    };

    let mut data = std::collections::HashMap::new();
    data.insert(
        format!("note:{}", note_path).into_bytes(),
        content.into_bytes(),
    );

    let mut tt = time_travel.lock().unwrap();
    let ts = tt.capture(data, &format!("manual-snapshot-{}", note_path));

    HttpResponse::Ok()
        .content_type("application/json")
        .json(json!({ "status": "ok", "timestamp": ts }))
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

/// `GET /search?q=...` — full-text search across all notes.
#[get("/search")]
async fn search_notes(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    query: web::Query<SearchQuery>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Read) {
        return e;
    }
    let max_results = query.limit.unwrap_or(20).min(100);
    match engine.search_notes(&query.q, max_results) {
        Ok(results) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "results": results, "query": query.q })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Search failed: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "search failed" }))
        }
    }
}

// ── Template handlers ───────────────────────────────────────────────────────

/// `GET /templates` — list all saved templates.
#[get("/templates")]
async fn list_templates_handler(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Read) {
        return e;
    }
    match crate::notes::template::list_templates(&engine) {
        Ok(templates) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "templates": templates })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to list templates: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Request body for `PUT /templates/{name}`.
#[derive(Deserialize)]
pub struct PutTemplateBody {
    content: String,
}

/// `PUT /templates/{name}` — save a template.
#[put("/templates/{name}")]
async fn save_template_handler(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
    body: web::Json<PutTemplateBody>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Write) {
        return e;
    }
    let name = path.into_inner();
    match crate::notes::template::save_template(&engine, &name, &body.content) {
        Ok(_) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "status": "ok", "name": name })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to save template: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// `DELETE /templates/{name}` — delete a template.
#[delete("/templates/{name}")]
async fn delete_template_handler(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Delete) {
        return e;
    }
    let name = path.into_inner();
    match crate::notes::template::delete_template(&engine, &name) {
        Ok(_) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "status": "ok" })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to delete template: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Request body for `POST /notes/daily`.
#[derive(Deserialize)]
pub struct CreateDailyNoteBody {
    template: Option<String>,
}

/// `POST /notes/daily` — create a daily note, optionally from a template.
#[post("/daily")]
async fn create_daily_note_handler(
    req: HttpRequest,
    engine: web::Data<NotesEngine>,
    body: web::Json<CreateDailyNoteBody>,
) -> impl Responder {
    if let Err(e) = crate::api::require_permission(&req, crate::api::Permission::Write) {
        return e;
    }
    match crate::notes::template::create_daily_note(&engine, body.template.as_deref()) {
        Ok(path) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "status": "ok", "path": path })),
        Err(e) => {
            tracing::error!(target: "apexstore::api::notes", "Failed to create daily note: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

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
            .service(get_graph)
            .service(get_version_history)
            .service(get_note_at_version)
            .service(delete_version)
            .service(restore_version)
            .service(create_snapshot)
            .service(create_daily_note_handler),
    )
    .service(search_notes)
    .service(list_tags)
    .service(get_notes_by_tag)
    .service(list_templates_handler)
    .service(save_template_handler)
    .service(delete_template_handler);
}
