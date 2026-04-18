use crate::core::engine::{DEFAULT_SCAN_LIMIT, MAX_SCAN_LIMIT};
use crate::LsmEngine;

pub struct ServerConfig;
impl ServerConfig {
    pub fn from_file(_path: &str) -> crate::infra::error::Result<Self> {
        Ok(Self)
    }
    pub fn from_env() -> Self {
        Self
    }
}

pub async fn start_server(_engine: LsmEngine, _config: ServerConfig) -> crate::infra::error::Result<()> {
    Ok(())
}

fn _check_api() {
    let _ = DEFAULT_SCAN_LIMIT;
    let _ = MAX_SCAN_LIMIT;
}
