use crate::infra::replication::ReplicationFrame;
use crate::LsmEngine;
use actix_web::{post, web, HttpResponse, Responder};
use serde_json::json;

/// Handler for `POST /admin/replicate`.
///
/// Receives a [`ReplicationFrame`] from a primary node and applies the
/// contained WAL records to the local engine.
#[post("/admin/replicate")]
async fn replicate(
    engine: web::Data<LsmEngine>,
    body: web::Json<ReplicationFrame>,
) -> impl Responder {
    let frame = body.into_inner();

    for record in &frame.records {
        let cf = record.column_family.as_deref().unwrap_or("default");

        let result = if record.is_range_tombstone() {
            let start = record.range_start.as_deref().unwrap_or(&record.key);
            let end = record.range_end.as_deref().unwrap_or(&[]);
            engine.delete_range_cf(cf, start, end)
        } else if record.is_deleted {
            engine.delete_cf(cf, record.key.as_slice())
        } else {
            engine.put_cf(cf, record.key.clone(), record.value.clone())
        };

        if let Err(e) = result {
            tracing::error!(
                target: "apexstore::api::replication",
                "Failed to apply replicated record: {:?}",
                e
            );
            return HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({
                    "error": format!("failed to apply record: {}", e)
                }));
        }
    }

    tracing::debug!(
        target: "apexstore::api::replication",
        "Applied {} replicated records (seq={})",
        frame.records.len(),
        frame.sequence
    );

    HttpResponse::Ok()
        .content_type("application/json")
        .json(json!({
            "status": "ok",
            "records_applied": frame.records.len(),
            "sequence": frame.sequence
        }))
}

/// Register replication-related routes.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(replicate);
}
