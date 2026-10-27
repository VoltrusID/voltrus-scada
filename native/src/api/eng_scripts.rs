//! The Scripts section's side of the REST API (`src/api/scripts.rs`, the
//! server's `scripts` feature, run by QuickJS): the scripts the flow
//! engine's Script nodes call, a test run with a JSON input, and each
//! script's executions. Every route is the server's `demo_protected`
//! (operator) group.

use anyhow::Result;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

use super::{Method, VoltrusApi};

/// `GET /api/v1/scripts`, `POST` creates.
pub const SCRIPTS: &str = "/api/v1/scripts";
/// How many executions the history asks for (the web client's 20).
pub const HISTORY_LIMIT: u32 = 20;
/// One MiB, the unit the memory limit is edited in.
pub const MIB: i64 = 1024 * 1024;

/// `/api/v1/scripts/{id}`: `PUT` updates, `DELETE` deletes.
pub fn script_path(id: i64) -> String {
    format!("{SCRIPTS}/{id}")
}

/// `POST /api/v1/scripts/{id}/test`.
pub fn test_path(id: i64) -> String {
    format!("{SCRIPTS}/{id}/test")
}

/// `GET /api/v1/scripts/{id}/executions?limit=`, newest first.
pub fn executions_path(id: i64, limit: u32) -> String {
    format!("{SCRIPTS}/{id}/executions?limit={limit}")
}

/// `models::Script`, as every scripts route answers it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Script {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub source: String,
    /// The engine's complaint when the source was last saved, cleared by a
    /// save that parses (the server refuses to run the script while it is
    /// set).
    #[serde(default)]
    pub syntax_error: Option<String>,
    pub cpu_limit_ms: i64,
    pub memory_limit_bytes: i64,
    pub enabled: bool,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// `models::ScriptExecution`: a test run (`trigger_type` `test`) or a flow's
/// Script node (`flow_id` set, no trigger type).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ScriptExecution {
    pub id: i64,
    pub script_id: i64,
    #[serde(default)]
    pub flow_id: Option<String>,
    #[serde(default)]
    pub trigger_type: Option<String>,
    #[serde(default)]
    pub duration_ms: i64,
    /// `success` or `error` (the web client also colours `timeout`).
    pub status: String,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub created_at: i64,
}

impl ScriptExecution {
    /// What started it: the trigger type, a flow, or (as the web client
    /// reads a run with neither) `manual`.
    pub fn trigger(&self) -> &str {
        match (&self.trigger_type, &self.flow_id) {
            (Some(trigger), _) if !trigger.is_empty() => trigger.as_str(),
            (_, Some(_)) => "flow",
            _ => "manual",
        }
    }

    pub fn succeeded(&self) -> bool {
        self.status == "success"
    }
}

/// `POST /api/v1/scripts/{id}/test`'s answer. A script saved with a syntax
/// error answers `{success: false, error}` only, without running.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
pub struct TestResult {
    pub success: bool,
    /// The JSON the script's `main(input)` returned (`{}` for nothing).
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// Absent when the script did not run.
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

/// `CreateScriptRequest`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CreateScript {
    pub name: String,
    pub source: String,
    pub cpu_limit_ms: i64,
    pub memory_limit_bytes: i64,
}

/// `UpdateScriptRequest`: only the fields sent change.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct UpdateScript {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_limit_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_limit_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// `TestScriptRequest`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestScript {
    pub input: serde_json::Value,
}

impl VoltrusApi {
    /// `GET /api/v1/scripts`, by name.
    pub async fn scripts(&self) -> Result<Vec<Script>> {
        self.get_json(SCRIPTS).await
    }

    /// `POST /api/v1/scripts` (201 with the new script).
    pub async fn create_script(&self, body: &CreateScript) -> Result<Script> {
        self.send_json(Method::POST, SCRIPTS, body).await
    }

    /// `PUT /api/v1/scripts/{id}`, the script as saved.
    pub async fn update_script(&self, id: i64, body: &UpdateScript) -> Result<Script> {
        self.send_json(Method::PUT, &script_path(id), body).await
    }

    /// `DELETE /api/v1/scripts/{id}` (204; its executions go with it).
    pub async fn delete_script(&self, id: i64) -> Result<()> {
        self.send_empty::<IgnoredAny>(Method::DELETE, &script_path(id))
            .await?;
        Ok(())
    }

    /// `POST /api/v1/scripts/{id}/test` with `input` as the script's input.
    pub async fn test_script(&self, id: i64, input: serde_json::Value) -> Result<TestResult> {
        self.send_json(Method::POST, &test_path(id), &TestScript { input })
            .await
    }

    /// `GET /api/v1/scripts/{id}/executions?limit=`, newest first.
    pub async fn script_executions(&self, id: i64, limit: u32) -> Result<Vec<ScriptExecution>> {
        self.get_json(&executions_path(id, limit)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_decodes_as_the_server_serializes_it() {
        // `GET /api/v1/scripts`: one fine, one saved with a syntax error.
        let body = r#"[{
            "id": 3, "name": "Scale power", "source": "function main(input) {\n  return input;\n}\n",
            "syntax_error": null, "cpu_limit_ms": 5000, "memory_limit_bytes": 33554432,
            "enabled": true, "created_at": 1761390000, "updated_at": 1761400000
        }, {
            "id": 4, "name": "Broken", "source": "function (",
            "syntax_error": "SyntaxError: function name expected\n    at <eval>:1:10\n",
            "cpu_limit_ms": 1000, "memory_limit_bytes": 1048576, "enabled": false,
            "created_at": 1761390001, "updated_at": 1761390001
        }]"#;
        let list: Vec<Script> = serde_json::from_str(body).unwrap();
        assert_eq!(list[0].memory_limit_bytes, 32 * MIB);
        assert!(list[0].enabled && list[0].syntax_error.is_none());
        assert!(!list[1].enabled);
        assert!(
            list[1]
                .syntax_error
                .as_deref()
                .is_some_and(|error| error.starts_with("SyntaxError"))
        );
    }

    #[test]
    fn executions_decode_and_name_their_trigger() {
        let body = r#"[{
            "id": 9, "script_id": 3, "flow_id": null, "trigger_type": "test",
            "duration_ms": 4, "status": "success", "error_message": null,
            "output": "{\"value\":42}", "created_at": 1761400100
        }, {
            "id": 8, "script_id": 3, "flow_id": "f-1", "trigger_type": null,
            "duration_ms": 1001, "status": "error",
            "error_message": "Script execution timed out (1000ms limit)",
            "output": null, "created_at": 1761400000
        }, {
            "id": 7, "script_id": 3, "flow_id": null, "trigger_type": null,
            "duration_ms": 0, "status": "timeout", "error_message": null,
            "output": null, "created_at": 1761390000
        }]"#;
        let list: Vec<ScriptExecution> = serde_json::from_str(body).unwrap();
        assert_eq!(list[0].trigger(), "test");
        assert!(list[0].succeeded());
        assert_eq!(list[0].output.as_deref(), Some(r#"{"value":42}"#));
        assert_eq!(list[1].trigger(), "flow");
        assert!(!list[1].succeeded());
        assert_eq!(list[2].trigger(), "manual");
    }

    #[test]
    fn a_test_answer_decodes_whether_or_not_the_script_ran() {
        let ran: TestResult = serde_json::from_str(
            r#"{"success": true, "output": "{\"value\":43}", "error": null, "duration_ms": 3}"#,
        )
        .unwrap();
        assert!(ran.success);
        assert_eq!(ran.duration_ms, Some(3));
        let failed: TestResult = serde_json::from_str(
            r#"{"success": false, "output": null, "error": "ReferenceError: x is not defined", "duration_ms": 1}"#,
        )
        .unwrap();
        assert_eq!(
            failed.error.as_deref(),
            Some("ReferenceError: x is not defined")
        );
        // A script saved with a syntax error is not run.
        let refused: TestResult =
            serde_json::from_str(r#"{"success": false, "error": "SyntaxError: unexpected token"}"#)
                .unwrap();
        assert_eq!(refused.duration_ms, None);
        assert_eq!(refused.output, None);
    }

    #[test]
    fn bodies_carry_only_what_the_server_reads() {
        let create = CreateScript {
            name: "Scale".into(),
            source: "function main(input) { return input; }".into(),
            cpu_limit_ms: 5000,
            memory_limit_bytes: 32 * MIB,
        };
        assert_eq!(
            serde_json::to_value(&create).unwrap(),
            serde_json::json!({
                "name": "Scale",
                "source": "function main(input) { return input; }",
                "cpu_limit_ms": 5000,
                "memory_limit_bytes": 33_554_432
            })
        );
        let update = UpdateScript {
            enabled: Some(false),
            ..UpdateScript::default()
        };
        assert_eq!(
            serde_json::to_value(&update).unwrap(),
            serde_json::json!({ "enabled": false })
        );
        assert_eq!(
            serde_json::to_value(TestScript {
                input: serde_json::json!({ "value": 42 })
            })
            .unwrap(),
            serde_json::json!({ "input": { "value": 42 } })
        );
        assert_eq!(script_path(3), "/api/v1/scripts/3");
        assert_eq!(test_path(3), "/api/v1/scripts/3/test");
        assert_eq!(
            executions_path(3, HISTORY_LIMIT),
            "/api/v1/scripts/3/executions?limit=20"
        );
    }
}
