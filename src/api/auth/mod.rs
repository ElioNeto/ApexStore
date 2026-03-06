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

pub use error::{AuthError, AuthResult};
pub use manager::TokenManager;
pub use middleware::bearer_validator;
pub use token::{ApiToken, Permission};
