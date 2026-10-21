// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The MCP server on rmcp: `tools/list`, `tools/call`, and the attachment resource. Each
//! call re-reads the policy, finds the running engines, runs the tool on a blocking thread
//! (the engine client is blocking) and appends an audit entry.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ListResourceTemplatesResult,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{json, Value};

use crate::audit::{AuditEntry, AuditLog};
use crate::config::DesktopConfig;
use crate::discovery::Engines;
use crate::policy::Policy;
use crate::tools::{self, Ctx, Output, Session};

pub const SERVER_NAME: &str = "tern";
const VERSION: &str = env!("CARGO_PKG_VERSION");

const INSTRUCTIONS: &str = "Tools for the user's email in Tern. Messages with `draft: true` are \
the user's own unsent drafts, not mail anyone received. To revise a draft, use update_draft \
rather than creating another one. What the tools may see and change is set by the user's MCP \
policy (read-only, read-write or read-write-send, and which accounts and folders).";

/// One server: a profile directory and what lives as long as the session.
pub struct App {
    pub config_dir: PathBuf,
    session: Session,
    audit: AuditLog,
    /// Serving `tern-mcp http`, whose calls follow `http.enabled` rather than `enabled`.
    http: bool,
}

impl App {
    pub fn new(config_dir: PathBuf) -> App {
        App {
            audit: AuditLog::new(&config_dir),
            config_dir,
            session: Session::default(),
            http: false,
        }
    }

    pub fn over_http(config_dir: PathBuf) -> App {
        App {
            http: true,
            ..App::new(config_dir)
        }
    }

    fn policy(&self, desktop: &DesktopConfig) -> Result<Policy, String> {
        let policy = Policy::load(&self.config_dir, desktop)?;
        Ok(if self.http { policy.for_http() } else { policy })
    }

    fn engines(&self, client: &str) -> Engines {
        crate::discovery::connect(&self.config_dir, &format!("tern-mcp/{VERSION} ({client})"))
    }

    /// Runs a tool and records it.
    pub fn run_tool(&self, client: &str, name: &str, args: &Value) -> Output {
        let start = Instant::now();
        let desktop = DesktopConfig::load(&self.config_dir);
        let out = match self.policy(&desktop) {
            Err(e) => Output {
                content: vec![json!({"type": "text", "text": json!({"error": e}).to_string()})],
                is_error: true,
                denied: true,
                account_id: None,
            },
            Ok(policy) => {
                let mut engines = self.engines(client);
                let no_engine =
                    crate::discovery::no_engine_message(&self.config_dir, &engines.problems);
                let mut ctx = Ctx {
                    backend: &mut engines,
                    policy: &policy,
                    desktop: &desktop,
                    config_dir: &self.config_dir,
                    session: &self.session,
                    no_engine,
                };
                tools::call(&mut ctx, name, args)
            }
        };
        let account = out.account_id.clone().or_else(|| {
            args.get("accountId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        let (outcome, summary) = match (out.is_error, out.denied) {
            (false, _) => ("ok", "ok".to_owned()),
            (true, denied) => {
                let msg = serde_json::from_str::<Value>(out.first_text())
                    .ok()
                    .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
                    .unwrap_or_default();
                (
                    if denied { "denied" } else { "error" },
                    format!("error: {msg}"),
                )
            }
        };
        self.audit.append(&AuditEntry::new(
            client,
            name,
            args,
            account,
            outcome,
            &summary,
            start.elapsed().as_millis() as u64,
        ));
        out
    }

    /// `resources/read` of an attachment, recorded as `resources/read`.
    pub fn read_resource(&self, client: &str, uri: &str) -> Result<Value, String> {
        let start = Instant::now();
        let desktop = DesktopConfig::load(&self.config_dir);
        let result = self.policy(&desktop).and_then(|policy| {
            let mut engines = self.engines(client);
            let no_engine =
                crate::discovery::no_engine_message(&self.config_dir, &engines.problems);
            let mut ctx = Ctx {
                backend: &mut engines,
                policy: &policy,
                desktop: &desktop,
                config_dir: &self.config_dir,
                session: &self.session,
                no_engine,
            };
            tools::read_attachment_resource(&mut ctx, uri).map_err(|e| match e {
                tools::ToolError::Denied(m) | tools::ToolError::Failed(m) => m,
            })
        });
        let (outcome, summary, account) = match &result {
            Ok((_, a)) => ("ok", "ok".to_owned(), a.clone()),
            Err(e) => ("error", format!("error: {e}"), None),
        };
        self.audit.append(&AuditEntry::new(
            client,
            "resources/read",
            &json!({"uri": uri}),
            account,
            outcome,
            &summary,
            start.elapsed().as_millis() as u64,
        ));
        result.map(|(v, _)| v)
    }
}

#[derive(Clone)]
pub struct Handler {
    app: Arc<App>,
}

impl Handler {
    pub fn new(app: Arc<App>) -> Handler {
        Handler { app }
    }
}

/// `clientInfo` from the session's `initialize`, as `name version`.
fn client_name(ctx: &RequestContext<RoleServer>) -> String {
    ctx.peer
        .peer_info()
        .map(|p| {
            let c = &p.client_info;
            if c.version.is_empty() {
                c.name.clone()
            } else {
                format!("{} {}", c.name, c.version)
            }
        })
        .unwrap_or_else(|| "unknown".into())
}

fn from_json<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ErrorData> {
    serde_json::from_value(v).map_err(|e| ErrorData::internal_error(e.to_string(), None))
}

pub fn server_config() -> ServerConfig {
    serde_json::from_value(json!({
        "protocolVersion": ProtocolVersion::LATEST,
        "capabilities": {"tools": {}, "resources": {}},
        "serverInfo": {"name": SERVER_NAME, "title": "Tern", "version": VERSION},
        "instructions": INSTRUCTIONS,
    }))
    .expect("a valid ServerConfig")
}

pub fn tools_list() -> Value {
    let tools: Vec<Value> = tools::definitions()
        .into_iter()
        .map(|d| json!({"name": d.name, "description": d.description, "inputSchema": d.schema}))
        .collect();
    json!({"tools": tools})
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        server_config()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        from_json(tools_list())
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let client = client_name(&context);
        let name = request.name.to_string();
        let args = Value::Object(request.arguments.unwrap_or_default());
        let app = self.app.clone();
        let out = tokio::task::spawn_blocking(move || app.run_tool(&client, &name, &args))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let result: CallToolResult =
            from_json(json!({"content": out.content, "isError": out.is_error}))?;
        Ok(result.into())
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        from_json(json!({"resourceTemplates": [{
            "uriTemplate": format!("{}://{{messageId}}/{{fileId}}", tools::attachment_uri_scheme()),
            "name": "attachment",
            "title": "Email attachment",
            "description": "Raw bytes of an email attachment. Discover URIs via the get_attachment tool.",
        }]}))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let client = client_name(&context);
        let app = self.app.clone();
        let uri = request.uri;
        let r = tokio::task::spawn_blocking(move || app.read_resource(&client, &uri))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        match r {
            Ok(v) => Ok(from_json::<ReadResourceResult>(v)?.into()),
            Err(e) => Err(ErrorData::resource_not_found(e, None)),
        }
    }
}

/// Serves MCP on stdin/stdout until the client closes it.
pub async fn serve_stdio(app: Arc<App>) -> Result<(), String> {
    use rmcp::ServiceExt;
    let service = Handler::new(app)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| format!("MCP handshake failed: {e}"))?;
    service
        .waiting()
        .await
        .map_err(|e| format!("MCP session ended with an error: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_list_parses_as_rmcp_types() {
        let list: ListToolsResult = serde_json::from_value(tools_list()).unwrap();
        let names: Vec<&str> = list.tools.iter().map(|t| t.name.as_ref()).collect();
        for n in [
            "list_accounts",
            "reply_to_thread",
            "update_draft",
            "delete_draft",
            "send_draft",
            "list_tracked_messages",
            "get_top_links",
            "list_scheduled_sends",
        ] {
            assert!(names.contains(&n), "{n}");
        }
        let _ = server_config();
    }

    #[test]
    fn calls_are_audited() {
        let tmp = tempfile::tempdir().unwrap();
        let app = App::new(tmp.path().to_path_buf());
        let out = app.run_tool("test 1", "list_accounts", &json!({}));
        assert!(out.is_error && out.first_text().contains("isn't running"));
        let out = app.run_tool(
            "test 1",
            "set_starred",
            &json!({"threadIds": ["t"], "starred": true, "body": "secret"}),
        );
        assert!(out.denied);
        let log = std::fs::read_to_string(tmp.path().join(crate::audit::AUDIT_FILE)).unwrap();
        let lines: Vec<Value> = log
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            (&lines[0]["outcome"], &lines[0]["client"]),
            (&json!("error"), &json!("test 1"))
        );
        assert_eq!(lines[1]["outcome"], "denied");
        assert!(!log.contains("secret"));
    }
}
