//! User journeys through the real `getkeys` binary, with the GetKeys app not running so the CLI
//! opens the vault itself.
use getkeys_core::{App, prefs::Prefs};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio},
};

const STRIPE: &str = "sk_test_journey_4f9a2c";
const TOKEN: &str = "tok-analytics-77b1e0";
const ORIGIN: &str = "chrome-extension://mhdeafblmlpikemoliokafdegcdhoeed/";

struct Journey(tempfile::TempDir);

impl Drop for Journey {
    fn drop(&mut self) {
        let path = self.0.path().join("vault.db");
        if let Ok(entry) = keyring::Entry::new("dev.getkeys.vault", &path.to_string_lossy()) {
            let _ = entry.delete_credential();
        }
    }
}

fn registry() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../registry/dist")
}

impl Journey {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
    fn dir(&self) -> &Path {
        self.0.path()
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_getkeys"));
        command
            .env("GETKEYS_DATA_DIR", self.dir())
            .env("GETKEYS_REGISTRY", registry())
            .env_remove("STRIPE_KEY")
            .env_remove("GK_TOKEN");
        command
    }
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Output {
        let mut child = self
            .command()
            .args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(text) = stdin {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        }
        child.wait_with_output().unwrap()
    }
    /// Runs a command that must succeed and returns its stdout.
    fn ok(&self, args: &[&str], stdin: Option<&str>) -> String {
        let output = self.run(args, stdin);
        assert!(
            output.status.success(),
            "getkeys {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn keys(&self, args: &[&str]) -> Vec<Value> {
        let mut all = vec!["ls", "--json"];
        all.extend(args);
        serde_json::from_str(&self.ok(&all, None)).unwrap()
    }
    /// Saves a custom key for Stripe in `backend` and an analytics key with two values in `web`.
    fn with_keys(self) -> Self {
        let saved = self.ok(
            &[
                "add",
                "--name",
                "Stripe test",
                "--custom",
                "STRIPE_KEY",
                "--group",
                "backend",
            ],
            Some(STRIPE),
        );
        assert_eq!(
            saved.trim(),
            "Saved Stripe test as getkeys://custom/stripe-test"
        );
        self.ok(
            &[
                "add",
                "--name",
                "Analytics",
                "--custom",
                "ANALYTICS_URL=https://analytics.example.test",
                "--custom",
                "ANALYTICS_TOKEN",
                "--group",
                "web",
            ],
            Some(TOKEN),
        );
        self
    }
    fn activity(&self) -> Vec<(String, String)> {
        App::open(self.dir().into(), registry())
            .unwrap()
            .snapshot()
            .unwrap()
            .activity
            .into_iter()
            .map(|a| (a.action, a.subject))
            .collect()
    }
}

#[test]
fn custom_keys_from_adding_to_running_programs_to_deleting() {
    let journey = Journey::new().with_keys();

    let table = journey.ok(&["ls"], None);
    assert!(table.starts_with("REF"), "{table}");
    for text in [
        "getkeys://custom/stripe-test",
        "Stripe test",
        "backend",
        "ok",
    ] {
        assert!(table.contains(text), "{text} missing from {table}");
    }
    assert!(!table.contains(STRIPE) && !table.contains(TOKEN));

    let keys = journey.keys(&[]);
    assert_eq!(keys.len(), 2);
    let stripe = keys
        .iter()
        .find(|k| k["name"] == "Stripe test")
        .unwrap()
        .clone();
    let mut shape: Vec<&str> = stripe
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    shape.sort();
    assert_eq!(
        shape,
        [
            "account",
            "fields",
            "groups",
            "id",
            "name",
            "plugin_id",
            "problem",
            "provider",
            "ref"
        ]
    );
    assert_eq!(stripe["ref"], "getkeys://custom/stripe-test");
    assert_eq!(stripe["provider"], "Custom");
    assert_eq!(stripe["plugin_id"], Value::Null);
    assert_eq!(stripe["account"], Value::Null);
    assert_eq!(stripe["problem"], Value::Null);
    assert_eq!(stripe["groups"], json!(["backend"]));
    let fields = stripe["fields"].as_array().unwrap();
    assert_eq!(fields.len(), 1);
    let mut field_shape: Vec<&String> = fields[0].as_object().unwrap().keys().collect();
    field_shape.sort();
    assert_eq!(field_shape, ["key", "label", "secret"]);
    assert_eq!(fields[0]["secret"], true);
    assert!(!serde_json::to_string(&keys).unwrap().contains(STRIPE));

    let backend = journey.keys(&["--group", "backend"]);
    assert_eq!(backend.len(), 1);
    assert_eq!(backend[0]["name"], "Stripe test");
    assert_eq!(journey.keys(&["--group", "web"])[0]["name"], "Analytics");
    assert_eq!(journey.keys(&["--provider", "custom"]).len(), 2);
    assert!(journey.keys(&["--provider", "github"]).is_empty());

    let hidden = journey.run(&["get", "getkeys://custom/stripe-test"], None);
    assert!(!hidden.status.success() && hidden.stdout.is_empty());
    assert_eq!(
        journey
            .ok(&["get", "getkeys://custom/stripe-test", "--show"], None)
            .trim(),
        STRIPE
    );
    let id = stripe["id"].as_str().unwrap();
    assert_eq!(journey.ok(&["get", id, "--show"], None).trim(), STRIPE);
    assert_eq!(
        journey
            .ok(
                &[
                    "get",
                    "getkeys://custom/analytics",
                    "--field",
                    "ANALYTICS_TOKEN",
                    "--show"
                ],
                None
            )
            .trim(),
        TOKEN
    );
    assert_eq!(
        journey
            .ok(
                &["get", "getkeys://custom/analytics/ANALYTICS_URL", "--show"],
                None
            )
            .trim(),
        "https://analytics.example.test"
    );
    assert!(
        !journey
            .run(&["get", "getkeys://custom/nothing", "--show"], None)
            .status
            .success()
    );

    let dotenv = journey.ok(&["env", "--group", "backend"], None);
    assert!(dotenv.contains(&format!("STRIPE_KEY={STRIPE}")), "{dotenv}");
    assert!(!dotenv.contains("ANALYTICS"));
    let file = journey.dir().join("web.env");
    journey.ok(
        &["env", "--group", "web", "-o", file.to_str().unwrap()],
        None,
    );
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains(TOKEN) && written.contains("ANALYTICS_URL="));
    assert!(!written.contains(STRIPE));
    let everything = journey.ok(&["env"], None);
    assert!(everything.contains(STRIPE) && everything.contains(TOKEN));

    // No shell: the argument reaches the program literally, and its exit code comes back.
    let child = journey.run(
        &[
            "run",
            "--key",
            "getkeys://custom/stripe-test",
            "--",
            "node",
            "-e",
            &format!(
                "if(process.env.STRIPE_KEY!=='{STRIPE}'||process.env.ANALYTICS_TOKEN)process.exit(1);console.log(process.argv[1]);process.exit(7)"
            ),
            "literal;echo unsafe",
        ],
        None,
    );
    assert_eq!(
        child.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(
        String::from_utf8(child.stdout).unwrap().trim(),
        "literal;echo unsafe"
    );

    // References in variables and in an env file are resolved for the program.
    let env_file = journey.dir().join("app.env");
    std::fs::write(
        &env_file,
        "# app settings\nURL=\"{{ getkeys://custom/analytics/ANALYTICS_URL }}\"\nPLAIN=value\n",
    )
    .unwrap();
    let mut command = journey.command();
    let resolved = command
        .env("GK_TOKEN", "getkeys://custom/analytics/ANALYTICS_TOKEN")
        .args([
            "run",
            "--group",
            "backend",
            "--env-file",
            env_file.to_str().unwrap(),
            "--",
            "node",
            "-e",
            "const e=process.env;console.log([e.GK_TOKEN,e.URL,e.PLAIN,e.STRIPE_KEY].join(' '))",
        ])
        .output()
        .unwrap();
    assert!(resolved.status.success());
    assert_eq!(
        String::from_utf8(resolved.stdout).unwrap().trim(),
        format!("{TOKEN} https://analytics.example.test value {STRIPE}")
    );

    let template = journey.dir().join("config.tpl");
    let output = journey.dir().join("config.yml");
    std::fs::write(
        &template,
        "stripe: {{ getkeys://custom/stripe-test }}\nanalytics: getkeys://custom/analytics/ANALYTICS_TOKEN\nkeep: plain\n",
    )
    .unwrap();
    journey.ok(
        &[
            "inject",
            "-i",
            template.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        format!("stripe: {STRIPE}\nanalytics: {TOKEN}\nkeep: plain\n")
    );

    assert_eq!(
        journey
            .ok(&["rm", "getkeys://custom/stripe-test"], None)
            .trim(),
        "Deleted"
    );
    let left = journey.keys(&[]);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["name"], "Analytics");
    assert!(
        !journey
            .run(&["get", "getkeys://custom/stripe-test", "--show"], None)
            .status
            .success()
    );
}

#[test]
fn backups_restore_into_a_new_vault() {
    let journey = Journey::new().with_keys();
    let backup = journey.dir().join("keys.age");
    let passphrase = "correct horse battery staple\n";
    journey.ok(&["backup", backup.to_str().unwrap()], Some(passphrase));
    let restored = Journey::new();
    restored.ok(&["restore", backup.to_str().unwrap()], Some(passphrase));
    assert_eq!(
        restored
            .ok(&["get", "getkeys://custom/stripe-test", "--show"], None)
            .trim(),
        STRIPE
    );
    assert!(
        !Journey::new()
            .run(
                &["restore", backup.to_str().unwrap()],
                Some("wrong passphrase\n")
            )
            .status
            .success()
    );
}

#[test]
fn providers_and_completions() {
    let journey = Journey::new();
    let providers = journey.ok(&["providers", "ls"], None);
    assert!(
        providers.starts_with("ID") && providers.contains("github"),
        "{providers}"
    );
    let script = journey.ok(&["completions", "powershell"], None);
    assert!(script.contains("getkeys") && script.contains("inject"));
}

struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Mcp {
    fn start(journey: &Journey) -> Self {
        let mut child = journey
            .command()
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut mcp = Self {
            child,
            stdin,
            stdout,
            next: 0,
        };
        let init = mcp.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "getkeys");
        mcp.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        mcp
    }
    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], id, "{reply}");
        reply
    }
    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }
    fn finish(mut self) {
        drop(self.stdin);
        assert!(self.child.wait().unwrap().success());
    }
}

#[test]
fn agents_use_keys_over_mcp_and_every_read_is_recorded() {
    let journey = Journey::new().with_keys();
    let mut mcp = Mcp::start(&journey);

    assert_eq!(mcp.request("ping", json!({}))["result"], json!({}));
    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "list_keys",
            "get_secret",
            "run_with_keys",
            "write_env_file",
            "refresh",
            "check_key",
            "rotate_key"
        ]
    );

    let listed = mcp.tool("list_keys", json!({}));
    assert_eq!(listed["result"]["isError"], false);
    let keys = &listed["result"]["structuredContent"]["keys"];
    assert_eq!(*keys, json!(journey.keys(&[])));
    assert!(!listed.to_string().contains(STRIPE));
    let web = mcp.tool("list_keys", json!({"group": "web"}));
    assert_eq!(
        web["result"]["structuredContent"]["keys"][0]["name"],
        "Analytics"
    );

    let secret = mcp.tool("get_secret", json!({"ref": "getkeys://custom/stripe-test"}));
    assert_eq!(secret["result"]["content"][0]["text"], STRIPE);
    let missing = mcp.tool("get_secret", json!({"ref": "getkeys://custom/nothing"}));
    assert_eq!(missing["result"]["isError"], true);

    let ran = mcp.tool(
        "run_with_keys",
        json!({
            "command": ["node", "-e", "console.log('key=' + process.env.STRIPE_KEY); console.error(process.env.ANALYTICS_TOKEN); process.exit(3)"],
            "keys": ["getkeys://custom/stripe-test"],
            "group": "web",
        }),
    );
    let output = &ran["result"]["structuredContent"];
    assert_eq!(output["exit_code"], 3);
    assert_eq!(output["stdout"].as_str().unwrap().trim(), "key=[redacted]");
    assert_eq!(output["stderr"].as_str().unwrap().trim(), "[redacted]");
    assert!(!ran.to_string().contains(STRIPE) && !ran.to_string().contains(TOKEN));

    let env_file = journey.dir().join("agent.env");
    let wrote = mcp.tool(
        "write_env_file",
        json!({"path": env_file.to_str().unwrap(), "group": "backend"}),
    );
    assert_eq!(wrote["result"]["isError"], false);
    assert!(
        std::fs::read_to_string(&env_file)
            .unwrap()
            .contains(&format!("STRIPE_KEY={STRIPE}"))
    );

    assert_eq!(
        mcp.request("resources/list", json!({}))["error"]["code"],
        -32601
    );
    assert_eq!(
        mcp.tool("delete_everything", json!({}))["error"]["code"],
        -32602
    );
    mcp.finish();

    let stripe_id = journey.keys(&["--group", "backend"])[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let activity = journey.activity();
    for action in ["mcp.get_secret", "mcp.run_with_keys", "mcp.write_env_file"] {
        assert!(
            activity.contains(&(action.into(), stripe_id.clone())),
            "{action} missing from {activity:?}"
        );
    }
}

#[test]
fn agents_are_refused_when_mcp_is_turned_off() {
    let journey = Journey::new().with_keys();
    App::open(journey.dir().into(), registry())
        .unwrap()
        .set_prefs(Prefs { mcp_enabled: false })
        .unwrap();
    let mut mcp = Mcp::start(&journey);
    assert!(mcp.request("tools/list", json!({}))["result"]["tools"].is_array());
    let refused = mcp.tool("get_secret", json!({"ref": "getkeys://custom/stripe-test"}));
    assert_eq!(refused["error"]["code"], -32000);
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Settings")
    );
    assert!(!refused.to_string().contains(STRIPE));
    mcp.finish();
    assert!(
        !journey
            .activity()
            .iter()
            .any(|(action, _)| action.starts_with("mcp."))
    );
}

fn frame(stdin: &mut ChildStdin, message: Value) {
    let bytes = serde_json::to_vec(&message).unwrap();
    stdin
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .unwrap();
    stdin.write_all(&bytes).unwrap();
}

fn unframe(stdout: &mut ChildStdout) -> Value {
    let mut length = [0u8; 4];
    stdout.read_exact(&mut length).unwrap();
    let mut message = vec![0; u32::from_le_bytes(length) as usize];
    stdout.read_exact(&mut message).unwrap();
    serde_json::from_slice(&message).unwrap()
}

#[test]
fn the_browser_extension_saves_keys_through_the_native_host() {
    let journey = Journey::new();
    let mut child = journey
        .command()
        .arg(ORIGIN)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut ask = |message: Value| {
        frame(&mut stdin, message);
        unframe(&mut stdout)
    };

    assert_eq!(
        ask(json!({"type": "ping"})),
        json!({"ok": true, "version": env!("CARGO_PKG_VERSION")})
    );
    let patterns = ask(json!({"type": "patterns"}));
    assert_eq!(patterns["ok"], true);
    let patterns = patterns["patterns"].as_array().unwrap();
    assert!(!patterns.is_empty());
    for p in patterns {
        assert!(p["provider"].as_str().unwrap().contains('.'));
        assert!(!p["name"].as_str().unwrap().is_empty());
        regex::Regex::new(p["pattern"].as_str().unwrap()).unwrap();
    }
    let saved = ask(json!({
        "type": "save",
        "provider": null,
        "name": "",
        "value": "  browser-captured-secret-91  ",
        "url": "https://platform.example.com/settings/keys",
    }));
    assert_eq!(saved["ok"], true, "{saved}");
    let id = saved["id"].as_str().unwrap().to_owned();
    let named = ask(
        json!({"type": "save", "provider": null, "name": "Mailer", "value": "mail-secret-55", "url": ""}),
    );
    assert_eq!(named["ok"], true, "{named}");
    let empty = ask(json!({"type": "save", "provider": null, "name": "x", "value": " "}));
    assert_eq!(empty["ok"], false);
    assert!(empty["error"].as_str().is_some());
    let unknown = ask(json!({"type": "reveal_everything"}));
    assert_eq!(unknown["ok"], false);
    drop(stdin);
    assert!(child.wait().unwrap().success());

    let keys = journey.keys(&[]);
    let key = keys.iter().find(|k| k["id"] == id.as_str()).unwrap();
    assert_eq!(key["name"], "Key from platform.example.com");
    assert_eq!(key["fields"][0]["key"], "API_KEY");
    assert_eq!(
        journey.ok(&["get", &id, "--show"], None).trim(),
        "browser-captured-secret-91"
    );
    assert!(keys.iter().any(|k| k["name"] == "Mailer"));
}

#[test]
fn the_native_host_refuses_other_extensions() {
    let journey = Journey::new();
    let refused = journey.run(&["chrome-extension://someoneelse/"], Some(""));
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
}
