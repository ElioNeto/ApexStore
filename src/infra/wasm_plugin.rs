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
//! wasm = ["dep:wasmtime"]
//! ```

use wasmtime::{Engine, Instance, Linker, Module, Store, Val, ValType};

/// A loaded WebAssembly plugin instance.
///
/// Holds a [`wasmtime::Engine`], [`wasmtime::Store`], and [`wasmtime::Instance`]
/// to provide actual WASM execution via `wasmtime`.
pub struct WasmPlugin {
    /// Human-readable name of the plugin (derived from the file stem).
    name: String,
    /// WASM engine (compilation environment).
    /// Kept alive to support the store and instance.
    #[allow(dead_code)]
    engine: Engine,
    /// Store holding the WASM linear memory and globals.
    store: Store<()>,
    /// Instantiated WASM module.
    instance: Instance,
}

impl WasmPlugin {
    /// Load a WASM module from a file path.
    ///
    /// Reads the file, compiles it with `wasmtime`, and instantiates the module.
    /// Returns an error if the file cannot be read, is not a valid WASM binary,
    /// or if instantiation fails.
    pub fn load<P: AsRef<std::path::Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let module_bytes = std::fs::read(path.as_ref())?;
        let name = path
            .as_ref()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed")
            .to_string();

        let engine = Engine::default();
        let module = Module::new(&engine, &module_bytes)?;
        let mut store = Store::new(&engine, ());
        let linker = Linker::new(&engine);
        let instance = linker.instantiate(&mut store, &module)?;

        Ok(Self {
            name,
            engine,
            store,
            instance,
        })
    }

    /// Call an exported function in the WASM module.
    ///
    /// `function_name` must match an exported function.
    /// `args` is a JSON-encoded array of numbers, e.g. `[1, 2]`.
    /// Each argument is converted to the WASM type expected by the function
    /// (i32, i64, f32, or f64). Returns the JSON-encoded result array.
    pub fn call(
        &mut self,
        function_name: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let func = self
            .instance
            .get_func(&mut self.store, function_name)
            .ok_or_else(|| {
                format!(
                    "function '{}' not found in plugin '{}'",
                    function_name, self.name
                )
            })?;

        let ty = func.ty(&self.store);
        let param_types: Vec<ValType> = ty.params().collect();
        let result_types: Vec<ValType> = ty.results().collect();

        // Parse JSON args as a generic JSON array
        let json_args: Vec<serde_json::Value> = serde_json::from_slice(args)?;

        if json_args.len() != param_types.len() {
            return Err(format!(
                "expected {} argument(s), got {}",
                param_types.len(),
                json_args.len()
            )
            .into());
        }

        // Convert JSON values to wasmtime Val based on the function's parameter types
        let mut wasm_args = Vec::with_capacity(param_types.len());
        for (val, ty) in json_args.iter().zip(param_types.iter()) {
            let wval = match ty {
                ValType::I32 => Val::I32(
                    val.as_i64()
                        .ok_or_else(|| format!("expected i32, got {}", val))?
                        as i32,
                ),
                ValType::I64 => Val::I64(
                    val.as_i64()
                        .ok_or_else(|| format!("expected i64, got {}", val))?,
                ),
                ValType::F32 => {
                    let f = val
                        .as_f64()
                        .ok_or_else(|| format!("expected f32, got {}", val))?
                        as f32;
                    Val::F32(f.to_bits())
                }
                ValType::F64 => {
                    let f = val
                        .as_f64()
                        .ok_or_else(|| format!("expected f64, got {}", val))?;
                    Val::F64(f.to_bits())
                }
                other => {
                    return Err(format!("unsupported WASM value type {:?}", other).into());
                }
            };
            wasm_args.push(wval);
        }

        // Prepare a buffer for the results (one default Val per result type)
        let mut wasm_results: Vec<Val> = result_types
            .iter()
            .map(|ty| match ty {
                ValType::I32 => Val::I32(0),
                ValType::I64 => Val::I64(0),
                ValType::F32 => Val::F32(0u32),
                ValType::F64 => Val::F64(0u64),
                _ => Val::I32(0),
            })
            .collect();

        func.call(&mut self.store, &wasm_args, &mut wasm_results)?;

        // Convert wasmtime values back to serde_json::Value
        let json_results: Vec<serde_json::Value> = wasm_results
            .iter()
            .map(|val| match val {
                Val::I32(n) => serde_json::Value::Number((*n).into()),
                Val::I64(n) => serde_json::Value::Number((*n).into()),
                Val::F32(n) => serde_json::Number::from_f64(f32::from_bits(*n) as f64)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
                Val::F64(n) => serde_json::Number::from_f64(f64::from_bits(*n))
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
                _ => serde_json::Value::Null,
            })
            .collect();

        Ok(serde_json::to_vec(&json_results)?)
    }

    /// Unload the WASM module and release all associated resources.
    ///
    /// After calling this method the plugin should not be used again.
    /// The underlying wasmtime resources (store, instance, engine) are
    /// released when the struct is dropped; this method clears the name
    /// as a safety measure.
    pub fn unload(&mut self) {
        self.name.clear();
    }

    /// Returns the plugin name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid WASM module that exports an `add(a: i32, b: i32) -> i32` function.
    const ADD_WASM: &[u8] = b"\0asm\x01\0\0\0\x01\x07\x01\x60\x02\x7f\x7f\x01\x7f\x03\x02\x01\0\x07\x07\x01\x03\x61\x64\x64\0\0\x0a\x09\x01\x07\0\x20\0\x20\x01\x6a\x0b";

    #[test]
    fn test_wasm_plugin_load_invalid_path() {
        let result = WasmPlugin::load("/nonexistent/plugin.wasm");
        assert!(result.is_err());
    }

    #[test]
    fn test_wasm_plugin_load_invalid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not_wasm.bin");
        std::fs::write(&path, b"not a wasm binary").unwrap();
        let result = WasmPlugin::load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_wasm_plugin_unload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("add.wasm");
        std::fs::write(&path, ADD_WASM).unwrap();

        let mut plugin = WasmPlugin::load(&path).unwrap();
        assert_eq!(plugin.name(), "add");
        plugin.unload();
        // After unload, name should be cleared
        assert_eq!(plugin.name(), "");
    }

    #[test]
    fn test_wasm_plugin_add_function() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("add.wasm");
        std::fs::write(&path, ADD_WASM).unwrap();

        let mut plugin = WasmPlugin::load(&path).unwrap();
        let result = plugin.call("add", b"[1, 2]").unwrap();
        let result_json: Vec<i32> = serde_json::from_slice(&result).unwrap();
        assert_eq!(result_json, vec![3]);
    }

    #[test]
    fn test_wasm_plugin_call_nonexistent_function() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("add.wasm");
        std::fs::write(&path, ADD_WASM).unwrap();

        let mut plugin = WasmPlugin::load(&path).unwrap();
        let result = plugin.call("nonexistent", b"[]");
        assert!(result.is_err());
    }

    #[test]
    fn test_wasm_plugin_call_wrong_args_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("add.wasm");
        std::fs::write(&path, ADD_WASM).unwrap();

        let mut plugin = WasmPlugin::load(&path).unwrap();
        // add expects 2 args, we pass 3
        let result = plugin.call("add", b"[1, 2, 3]");
        assert!(result.is_err());
    }
}
