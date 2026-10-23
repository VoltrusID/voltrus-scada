//! Sandboxed script engine for flow-engine Script nodes.
//!
//! Backed by QuickJS (`rquickjs`) — replaces the previous `boa_engine` backend
//! (boa pulled ~70 MB RSS; QuickJS is ~2 MB). The public interface is unchanged:
//! `execute_script` / `validate_syntax` / `ScriptResult`, so the flow-engine
//! Script node and its tests need no changes.
//!
//! Per-call model (matches the previous boa backend): spawn a std thread, build a
//! fresh QuickJS `Runtime` + `Context` on it (QuickJS runtimes are `!Send`), and
//! enforce the CPU deadline via `recv_timeout` on the caller side plus a runtime
//! interrupt handler on the worker side (so infinite loops actually terminate).
//!
//! Sandbox: no `require`/`import`/network/global access (string-blocked up front,
//! same rules as before); QuickJS `Context::full` provides only ES intrinsics.
//! Input is injected as a global `input` object (via `JSON.parse`); a script may
//! define `main(input)` whose return is JSON-serialized as the output.

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ScriptResult {
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

#[cfg(feature = "scripts")]
use std::time::{Duration, Instant};

/// Validate JS syntax. Returns None if valid.
#[cfg(feature = "scripts")]
pub fn validate_syntax(source: &str) -> Option<String> {
    if source.trim().is_empty() {
        return Some("Script source is empty".to_string());
    }
    use rquickjs::{Context, Runtime, Value};
    let runtime = match Runtime::new() {
        Ok(r) => r,
        Err(e) => return Some(format!("runtime init failed: {e}")),
    };
    let ctx = match Context::full(&runtime) {
        Ok(c) => c,
        Err(e) => return Some(format!("context init failed: {e}")),
    };
    ctx.with(|c| -> Option<String> {
        match c.eval::<Value, _>(source) {
            Ok(_) => None,
            Err(e) => Some(format!("{e}")),
        }
    })
}

/// Validate JS syntax — stub when scripts feature is disabled.
#[cfg(not(feature = "scripts"))]
pub fn validate_syntax(_source: &str) -> Option<String> {
    Some("Script engine is not available (scripts feature disabled)".to_string())
}

/// Execute a script with the given input and resource limits.
#[cfg(feature = "scripts")]
pub fn execute_script(
    source: &str,
    input: &Value,
    cpu_limit_ms: u64,
    memory_limit_bytes: u64,
) -> ScriptResult {
    let start = Instant::now();
    let input_json = input.to_string();
    let result = execute_with_timeout(source, &input_json, cpu_limit_ms, memory_limit_bytes);
    let duration_ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(output) => ScriptResult {
            success: true,
            output: Some(output),
            error: None,
            duration_ms,
        },
        Err(e) => ScriptResult {
            success: false,
            output: None,
            error: Some(e),
            duration_ms,
        },
    }
}

/// Execute a script — stub when scripts feature is disabled.
#[cfg(not(feature = "scripts"))]
pub fn execute_script(
    _source: &str,
    _input: &Value,
    _cpu_limit_ms: u64,
    _memory_limit_bytes: u64,
) -> ScriptResult {
    ScriptResult {
        success: false,
        output: None,
        error: Some("Script engine is not available (scripts feature disabled)".to_string()),
        duration_ms: 0,
    }
}

#[cfg(feature = "scripts")]
fn execute_with_timeout(
    source: &str,
    input_json: &str,
    cpu_limit_ms: u64,
    memory_limit_bytes: u64,
) -> Result<String, String> {
    // Security pattern blocking (unchanged from the boa backend).
    if source.contains("require(") || source.contains("import ") {
        return Err("Scripts cannot use require() or import statements".to_string());
    }
    if source.contains("fetch(") || source.contains("XMLHttpRequest") {
        return Err("Scripts cannot make network requests".to_string());
    }
    if source.contains("process.") || source.contains("global.") {
        return Err("Scripts cannot access process or global objects".to_string());
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let source = source.to_string();
    let input_json = input_json.to_string();

    std::thread::spawn(move || {
        let result = run_script_in_quickjs(&source, &input_json, memory_limit_bytes, cpu_limit_ms);
        let _ = tx.send(result);
    });

    let timeout = Duration::from_millis(cpu_limit_ms);
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "Script execution timed out after {}ms",
            cpu_limit_ms
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err("Script execution thread panicked".to_string())
        }
    }
}

/// Run a script on a fresh QuickJS runtime (worker thread).
///
/// Contract (preserved from the boa backend):
/// - `input` is a global holding the parsed JSON input.
/// - If `main` is defined, its return is the result; otherwise `undefined`.
/// - `undefined`/`null` result → output `"{}"`; otherwise JSON-stringified.
#[cfg(feature = "scripts")]
fn run_script_in_quickjs(
    source: &str,
    input_json: &str,
    memory_limit_bytes: u64,
    cpu_limit_ms: u64,
) -> Result<String, String> {
    use rquickjs::{Context, Runtime};

    let runtime = Runtime::new().map_err(|e| format!("runtime init failed: {e}"))?;
    runtime.set_memory_limit(memory_limit_bytes as usize);
    runtime.set_max_stack_size(2 * 1024 * 1024);

    // Interrupt handler: abort (return true) once the deadline has passed.
    let deadline = Instant::now() + Duration::from_millis(cpu_limit_ms.max(1));
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));

    let ctx = Context::full(&runtime).map_err(|e| format!("context init failed: {e}"))?;

    ctx.with(|c| -> Result<String, String> {
        // Build the full program: inject `input` via JSON.parse, run the user
        // source, then call `main(input)` if defined and JSON.stringify its result.
        let input_lit = js_string_literal(input_json);
        let program = format!(
            "(function () {{\n\
             \x20\x20\x20\x20var input = JSON.parse({input_lit});\n\
             \x20\x20\x20\x20{source}\n\
             \x20\x20\x20\x20var __r = (typeof main === 'function') ? main(input) : undefined;\n\
             \x20\x20\x20\x20return (__r === undefined || __r === null) ? '{{}}' : JSON.stringify(__r);\n\
             }})()",
            input_lit = input_lit,
            source = source
        );
        let out: String = c
            .eval::<String, _>(program.as_str())
            .map_err(|e| format!("Script error: {e}"))?;
        Ok(out)
    })
}

/// Embed an arbitrary string as a double-quoted JS string literal.
#[cfg(feature = "scripts")]
fn js_string_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_syntax_empty() {
        assert!(validate_syntax("").is_some());
        assert!(validate_syntax("   ").is_some());
    }

    #[test]
    fn test_execute_blocks_require() {
        let source = "function main() { require('fs'); }";
        let input = json!({});
        let result = execute_script(source, &input, 1000, 1048576);
        assert!(!result.success);
        assert!(result.error.unwrap().contains("require"));
    }

    #[test]
    fn test_execute_blocks_fetch() {
        let source = "function main() { fetch('http://evil.com'); }";
        let input = json!({});
        let result = execute_script(source, &input, 1000, 1048576);
        assert!(!result.success);
        assert!(result.error.unwrap().contains("network"));
    }

    #[test]
    fn test_execute_blocks_process() {
        let source = "function main() { process.exit(1); }";
        let input = json!({});
        let result = execute_script(source, &input, 1000, 1048576);
        assert!(!result.success);
        assert!(result.error.unwrap().contains("process"));
    }

    #[cfg(feature = "scripts")]
    #[test]
    fn test_validate_syntax_valid() {
        let source = "function main(input) { return { outputs: {} }; }";
        assert_eq!(validate_syntax(source), None);
    }

    #[cfg(feature = "scripts")]
    #[test]
    fn test_validate_syntax_unbalanced_braces() {
        let source = "function main() { return 1; ";
        assert!(validate_syntax(source).is_some());
    }

    #[cfg(feature = "scripts")]
    #[test]
    fn test_execute_main_returns_output() {
        let source = "function main(input) { return { doubled: input.x * 2 }; }";
        let input = json!({ "x": 21 });
        let result = execute_script(source, &input, 1000, 1048576);
        assert!(result.success, "err: {:?}", result.error);
        let out: Value = serde_json::from_str(&result.output.unwrap()).unwrap();
        assert_eq!(out["doubled"], 42);
    }

    #[cfg(feature = "scripts")]
    #[test]
    fn test_execute_no_main_returns_empty() {
        let source = "var x = 1 + 1;";
        let input = json!({});
        let result = execute_script(source, &input, 1000, 1048576);
        assert!(result.success);
        assert_eq!(result.output.unwrap(), "{}");
    }
}
