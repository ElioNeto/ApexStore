//! Admin API module — dashboard and management endpoints.

pub mod dashboard;

use actix_web::web;

/// Register admin API routes.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(dashboard::admin_dashboard);
}
