//! Authentication module for ApexStore API
//!
//! Implements Bearer Token authentication with:
//! - Token generation and validation
//! - Middleware for request authentication
//! - Token management (CRUD operations)
//! - Permission-based access control

pub mod error;
pub mod manager;
pub mod middleware;
pub mod token;

/// Whether Bearer token authentication is enforced.
///
/// Registered in app data by `start_server`. This is a newtype rather than a
/// bare `bool` because actix-web keys app data by `TypeId`: two
/// `web::Data<bool>` registrations silently overwrite each other, and the
/// last one registered wins for *every* reader. Sharing `Data<bool>` with the
/// access-control flag previously made `API_AUTH_ENABLED` read the value of
/// `ACCESS_CONTROL_ENABLED`, disabling authentication outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthEnabled(pub bool);

impl AuthEnabled {
    /// Returns `true` when authentication must be enforced.
    pub fn is_enabled(&self) -> bool {
        self.0
    }
}

pub use error::{AuthError, AuthResult};
pub use manager::TokenManager;
pub use middleware::{bearer_validator, require_permission};
pub use token::{ApiToken, Permission};
