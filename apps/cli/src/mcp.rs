//! `getkeys mcp`: a Model Context Protocol server on stdin/stdout for AI agents.
use crate::{client::Client, keys, ops};
use anyhow::{Context, Result, bail};
use getkeys_core::{commands::Command, model::CredentialView, prefs::Prefs};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

const PROTOCOL: &str = "2025-06-18";
const OUTPUT_LIMIT: usize = 50_000;

pub fn serve(client: &mut Client) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(client, &message),
            Err(_) => Some(error(Value::Null, -32700, "Parse error")),
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn handle(client: &mut Client, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        // Replies to requests this server never sends.
        if message.get("result").is_some() || message.get("error").is_some() {
            return None;
        }
        return Some(error(id.unwrap_or_default(), -32600, "Invalid Request"));
    };
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error(id.unwrap_or_default(), -32600, "Invalid Request"));
    }
    // Notifications such as notifications/initialized get no reply.
    let id = id?;
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "getkeys", "title": "GetKeys", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Use the keys in the person's GetKeys vault. Prefer run_with_keys or write_env_file, which keep values out of the conversation, over get_secret."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call(client, &params),
        _ => Err((-32601, format!("Method not found: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => error(id, code, &message),
    })
}

fn tool(name: &str, title: &str, description: &str, input: Value, hints: Value) -> Value {
    let mut annotations = hints;
    annotations["title"] = json!(title);
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input,
        "annotations": annotations,
    })
}

fn tools() -> Vec<Value> {
    let selection = |extra: Value| {
        let mut properties = json!({
            "keys": {"type": "array", "items": {"type": "string"}, "description": "References (getkeys://provider/key) or IDs from list_keys"},
            "group": {"type": "string", "description": "Use every key in this group"}
        });
        if let (Some(p), Some(extra)) = (properties.as_object_mut(), extra.as_object()) {
            p.extend(extra.clone());
        }
        properties
    };
    let reference = json!({"type": "object", "properties": {"ref": {"type": "string", "description": "getkeys://provider/key reference or key ID"}}, "required": ["ref"]});
    vec![
        tool(
            "list_keys",
            "List keys",
            "Lists the keys in GetKeys with their references, providers, groups, fields and problems. Never includes values.",
            json!({"type": "object", "properties": {"group": {"type": "string"}, "provider": {"type": "string"}}}),
            json!({"readOnlyHint": true, "openWorldHint": false}),
        ),
        tool(
            "get_secret",
            "Get a secret value",
            "Returns one field's raw value. Prefer run_with_keys or write_env_file so the value stays out of the conversation.",
            json!({"type": "object", "properties": {"ref": {"type": "string", "description": "getkeys://provider/key[/field] reference or key ID"}, "field": {"type": "string"}}, "required": ["ref"]}),
            json!({"readOnlyHint": true, "openWorldHint": false}),
        ),
        tool(
            "run_with_keys",
            "Run with keys",
            "Runs a program (no shell) with the chosen keys as environment variables, named as GetKeys exports them. Returns the exit code and output with secret values redacted.",
            json!({"type": "object", "properties": selection(json!({
                "command": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Program and arguments"},
                "cwd": {"type": "string", "description": "Working directory"}
            })), "required": ["command"]}),
            json!({"readOnlyHint": false, "destructiveHint": true, "openWorldHint": true}),
        ),
        tool(
            "write_env_file",
            "Write a .env file",
            "Writes the chosen keys (all keys when none are chosen) to a .env file, replacing it.",
            json!({"type": "object", "properties": selection(json!({"path": {"type": "string"}})), "required": ["path"]}),
            json!({"readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": false}),
        ),
        tool(
            "refresh",
            "Refresh keys",
            "Looks for new keys on this device and lists the keys in each connected account, or only those in a group.",
            json!({"type": "object", "properties": {"group": {"type": "string"}}}),
            json!({"readOnlyHint": false, "destructiveHint": false, "openWorldHint": true}),
        ),
        tool(
            "check_key",
            "Check a key",
            "Asks the provider whether a key still works.",
            reference.clone(),
            json!({"readOnlyHint": true, "openWorldHint": true}),
        ),
        tool(
            "rotate_key",
            "Rotate a key",
            "Has the provider replace the key with a new one. The old value stops working.",
            reference,
            json!({"readOnlyHint": false, "destructiveHint": true, "openWorldHint": true}),
        ),
    ]
}

fn call(client: &mut Client, params: &Value) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "Missing tool name".to_owned()))?;
    if !tools().iter().any(|t| t["name"] == name) {
        return Err((-32602, format!("Unknown tool: {name}")));
    }
    let prefs: Prefs = client
        .get(Command::GetPrefs)
        .map_err(|e| (-32603, format!("{e:#}")))?;
    if !prefs.mcp_enabled {
        return Err((
            -32000,
            "Agent access is turned off in GetKeys. Turn on MCP in GetKeys Settings › Connect to an agent.".to_owned(),
        ));
    }
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    Ok(match run(client, name, &args) {
        Ok((text, structured)) => {
            let mut result = json!({"content": [{"type": "text", "text": text}], "isError": false});
            if let Some(s) = structured {
                result["structuredContent"] = s;
            }
            result
        }
        Err(e) => json!({"content": [{"type": "text", "text": format!("{e:#}")}], "isError": true}),
    })
}

fn text(args: &Value, name: &str) -> Option<String> {
    args.get(name).and_then(Value::as_str).map(str::to_owned)
}

fn strings(args: &Value, name: &str) -> Vec<String> {
    args.get(name)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn clip(text: &str) -> String {
    if text.len() <= OUTPUT_LIMIT {
        return text.to_owned();
    }
    let mut end = OUTPUT_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[output truncated]", &text[..end])
}

fn find_key(client: &mut Client, args: &Value) -> Result<String> {
    let reference = text(args, "ref").context("Give ref")?;
    let snapshot = client.snapshot()?;
    let keys = keys::keys(&snapshot);
    Ok(keys::find(&keys, &reference)?.0.c.id.clone())
}

fn run(client: &mut Client, name: &str, args: &Value) -> Result<(String, Option<Value>)> {
    let groups: Vec<String> = text(args, "group").into_iter().collect();
    Ok(match name {
        "list_keys" => {
            let keys = crate::list(client, text(args, "group"), text(args, "provider"))?;
            (
                serde_json::to_string_pretty(&keys)?,
                Some(json!({ "keys": keys })),
            )
        }
        "get_secret" => {
            let reference = text(args, "ref").context("Give ref")?;
            let field = text(args, "field");
            let (value, _) =
                ops::value(client, &reference, field.as_deref(), Some("mcp.get_secret"))?;
            (value, None)
        }
        "run_with_keys" => {
            let command = strings(args, "command");
            let Some((program, rest)) = command.split_first() else {
                bail!("Give the command to run");
            };
            let env = ops::environment(
                client,
                &groups,
                &strings(args, "keys"),
                None,
                Some("mcp.run_with_keys"),
            )?;
            let mut child = std::process::Command::new(ops::program(program));
            child
                .args(rest)
                .envs(&env.vars)
                .stdin(std::process::Stdio::null());
            if let Some(cwd) = text(args, "cwd") {
                child.current_dir(cwd);
            }
            let output = child
                .output()
                .with_context(|| format!("Unable to start {program}"))?;
            let stdout = clip(&ops::redact(
                &String::from_utf8_lossy(&output.stdout),
                &env.secrets,
            ));
            let stderr = clip(&ops::redact(
                &String::from_utf8_lossy(&output.stderr),
                &env.secrets,
            ));
            let code = output.status.code();
            (
                format!(
                    "Exit code: {}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
                    code.map_or("none".into(), |c| c.to_string())
                ),
                Some(json!({"exit_code": code, "stdout": stdout, "stderr": stderr})),
            )
        }
        "write_env_file" => {
            let path = text(args, "path").context("Give path")?;
            let dotenv = ops::dotenv(
                client,
                &groups,
                &strings(args, "keys"),
                Some("mcp.write_env_file"),
            )?;
            std::fs::write(&path, &dotenv).with_context(|| format!("Unable to write {path}"))?;
            (
                format!("Wrote {} variables to {path}", dotenv.lines().count()),
                None,
            )
        }
        "refresh" => (
            ops::refresh(client, text(args, "group").as_deref(), &mut |p| {
                eprintln!("{p}")
            })?,
            None,
        ),
        "check_key" => {
            let id = find_key(client, args)?;
            let problem: Option<String> = client.get(Command::Check { credential_id: id })?;
            (problem.unwrap_or_else(|| "The key works".into()), None)
        }
        "rotate_key" => {
            let id = find_key(client, args)?;
            let c: CredentialView = client.get(Command::RotateNow { credential_id: id })?;
            (
                format!("Rotated {}. GetKeys holds the new value.", c.name),
                None,
            )
        }
        _ => bail!("Unknown tool: {name}"),
    })
}
