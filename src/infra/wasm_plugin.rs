//! WebAssembly plugin system — load and call WASM plugins at runtime.
//!
//! This module provides a [`WasmPlugin`] struct that can load a WebAssembly
//! module from a file, call exported functions by name, and unload the module
//! when no longer needed.
//!
//! # Feature gate
//!
//! This module is only available when the `wasm` feature is enabled.
//!
//! ```toml
//! [features]
//! wasm = []
//! ```

#[cfg(feature = "wasm")]
use std::collections::HashMap;

/// A loaded WebAssembly plugin instance.
///
/// Holds the raw bytes of the WASM module (a future implementation would
/// use `wasmtime` or `wasmer` to instantiate the module and call functions).
pub struct WasmPlugin {
    /// Human-readable name of the plugin.
    name: String,
    /// Raw WASM binary bytes.
    #[cfg(feature = "wasm")]
    module_bytes: Vec<u8>,
    /// Cached exports discovered at load time.
    #[cfg(feature = "wasm")]
    exports: HashMap<String, Vec<u8>>,
}

impl WasmPlugin {
    /// Load a WASM module from a file path.
    ///
    /// Reads the file into memory and discovers exported function names.
    /// Returns an error if the file cannot be read or does not contain
    /// a valid WASM binary.
    #[cfg(feature = "wasm")]
    pub fn load<P: AsRef<std::path::Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let module_bytes = std::fs::read(path.as_ref())?;
        let name = path
            .as_ref()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed")
            .to_string();

        // Minimal WASM binary validation: check magic bytes.
        if module_bytes.len() < 8 || &module_bytes[0..4] != b"\0asm" {
            return Err(format!("{} is not a valid WASM binary", path.as_ref().display()).into());
        }

        // Stub: discover exports from the WASM binary.
        // In a full implementation this would use wasmtime::Module::new().
        let exports = HashMap::new();

        Ok(Self {
            name,
            module_bytes,
            exports,
        })
    }

    /// Load a WASM module (no-op stub when `wasm` feature is disabled).
    #[cfg(not(feature = "wasm"))]
    pub fn load<P: AsRef<std::path::Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let _ = path;
        Err("WASM support is not enabled (compile with --features wasm)".into())
    }

    /// Call an exported function in the WASM module.
    ///
    /// `function_name` must match an exported function.
    /// `args` is a JSON-encoded array of arguments.
    /// Returns the JSON-encoded result.
    ///
    /// # Stub
    ///
    /// This is a stub that returns an error indicating WASM execution is not
    /// yet implemented. A full implementation would use `wasmtime::Func::call`.
    #[cfg(feature = "wasm")]
    pub fn call(
        &self,
        function_name: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let _ = (function_name, args);
        Err(format!(
            "WASM execution not yet implemented (plugin: {}, function: {})",
            self.name, function_name
        )
        .into())
    }

    /// Call an exported function (no-op stub when `wasm` feature is disabled).
    #[cfg(not(feature = "wasm"))]
    pub fn call(
        &self,
        function_name: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let _ = (function_name, args);
        Err("WASM support is not enabled (compile with --features wasm)".into())
    }

    /// Unload the WASM module and release all associated resources.
    ///
    /// After calling this method the plugin should not be used again.
    pub fn unload(&mut self) {
        #[cfg(feature = "wasm")]
        {
            self.module_bytes.clear();
            self.exports.clear();
        }
    }

    /// Returns the plugin name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_plugin_load_invalid_path() {
        let result = WasmPlugin::load("/nonexistent/plugin.wasm");
        assert!(result.is_err());
    }

    #[test]
    fn test_wasm_plugin_load_invalid_file() {
        // Create a temp file that is not a valid WASM binary
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not_wasm.bin");
        std::fs::write(&path, b"not a wasm binary").unwrap();
        let result = WasmPlugin::load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_wasm_plugin_unload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.wasm");
        // Write valid WASM header (magic + version) to pass validation
        std::fs::write(&path, b"\0asm\x01\0\0\0").unwrap();

        let result = WasmPlugin::load(&path);
        #[cfg(feature = "wasm")]
        {
            let mut plugin = result.unwrap();
            assert_eq!(plugin.name(), "empty");
            plugin.unload();
            // After unload, internal state should be cleared
        }
        #[cfg(not(feature = "wasm"))]
        {
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_wasm_plugin_call_fails_not_implemented() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wasm");
        std::fs::write(&path, b"\0asm\x01\0\0\0").unwrap();

        #[cfg(feature = "wasm")]
        {
            let plugin = WasmPlugin::load(&path).unwrap();
            let result = plugin.call("add", b"[1, 2]");
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("not yet implemented"));
        }
    }
}
