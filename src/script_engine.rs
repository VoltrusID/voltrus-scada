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

/// Fewest and most milliseconds of CPU a script may be given.
pub const CPU_LIMIT_MS: std::ops::RangeInclusive<i64> = 1..=10_000;
/// Smallest and largest QuickJS heap a script may be given, in bytes.
pub const MEMORY_LIMIT_BYTES: std::ops::RangeInclusive<i64> = 65_536..=67_108_864;

/// Stored limits clamped to [`CPU_LIMIT_MS`] and [`MEMORY_LIMIT_BYTES`]: a
/// negative value (which used to become "unlimited" when cast to `u64`) takes
/// the smallest allowed, a huge one the largest.
pub fn clamp_limits(cpu_limit_ms: i64, memory_limit_bytes: i64) -> (i64, i64) {
    (
        cpu_limit_ms.clamp(*CPU_LIMIT_MS.start(), *CPU_LIMIT_MS.end()),
        memory_limit_bytes.clamp(*MEMORY_LIMIT_BYTES.start(), *MEMORY_LIMIT_BYTES.end()),
    )
}

/// [`execute_script`] with the limits clamped, on a blocking thread so the
/// async runtime keeps serving while the script runs.
pub async fn run_script(
    source: String,
    input: Value,
    cpu_limit_ms: i64,
    memory_limit_bytes: i64,
) -> ScriptResult {
    let (cpu, mem) = clamp_limits(cpu_limit_ms, memory_limit_bytes);
    tokio::task::spawn_blocking(move || execute_script(&source, &input, cpu as u64, mem as u64))
        .await
        .unwrap_or_else(|e| ScriptResult {
            success: false,
            output: None,
            error: Some(format!("Script execution failed: {e}")),
            duration_ms: 0,
        })
}

/// [`validate_syntax`] on a blocking thread.
pub async fn check_syntax(source: String) -> Option<String> {
    tokio::task::spawn_blocking(move || validate_syntax(&source))
        .await
        .unwrap_or_else(|e| Some(format!("syntax check failed: {e}")))
}

/// Check JS syntax without running anything: the source is compiled only
/// (QuickJS `JS_EVAL_FLAG_COMPILE_ONLY`), under a memory limit. Returns None
/// if it compiles.
#[cfg(feature = "scripts")]
pub fn validate_syntax(source: &str) -> Option<String> {
    if source.trim().is_empty() {
        return Some("Script source is empty".to_string());
    }
    use rquickjs::{Context, Runtime, qjs};
    let Ok(c_source) = std::ffi::CString::new(source) else {
        return Some("Script source contains a NUL character".to_string());
    };
    let runtime = match Runtime::new() {
        Ok(r) => r,
        Err(e) => return Some(format!("runtime init failed: {e}")),
    };
    runtime.set_memory_limit(*MEMORY_LIMIT_BYTES.end() as usize);
    runtime.set_max_stack_size(2 * 1024 * 1024);
    let ctx = match Context::full(&runtime) {
        Ok(c) => c,
        Err(e) => return Some(format!("context init failed: {e}")),
    };
    ctx.with(|c| -> Option<String> {
        let flags = (qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as _;
        // SAFETY: the context is live for this closure; the source and file
        // name are NUL-terminated and outlive the call; the returned value is
        // owned and handed to `Value`, which frees it on drop.
        let compiled = unsafe {
            let raw = qjs::JS_Eval(
                c.as_raw().as_ptr(),
                c_source.as_ptr(),
                source.len() as _,
                c"<script>".as_ptr(),
                flags,
            );
            rquickjs::Value::from_raw(c.clone(), raw)
        };
        if !compiled.is_exception() {
            return None;
        }
        let caught = c.catch();
        Some(match caught.as_exception() {
            Some(e) => e
                .message()
                .map(|m| format!("SyntaxError: {m}"))
                .unwrap_or_else(|| e.to_string()),
            None => "SyntaxError".to_string(),
        })
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
    fn validate_syntax_compiles_without_running() {
        // Running this would never end; compiling it is instant.
        let started = Instant::now();
        assert_eq!(validate_syntax("while (true) {}"), None);
        assert!(started.elapsed() < Duration::from_secs(2));
        // A top-level throw is not a syntax error: nothing runs.
        assert_eq!(validate_syntax("throw new Error('x')"), None);
    }

    #[test]
    fn limits_are_clamped() {
        assert_eq!(clamp_limits(-1, -1), (1, 65_536));
        assert_eq!(clamp_limits(i64::MAX, i64::MAX), (10_000, 67_108_864));
        assert_eq!(clamp_limits(1000, 1_048_576), (1000, 1_048_576));
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
