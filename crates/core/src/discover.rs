//! Finds keys and sign-ins that already exist on this device, as each provider's manifest
//! describes: environment variables, CLI session files, commands, and OS keychains.
use crate::{model::AuthMaterial, now};
use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use getkeys_plugin_api::{FileFormat, Manifest, SecretValue, Source, SourceKind, SourceRole};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, process::Command};
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct Discovery {
    pub id: String,
    pub plugin_id: String,
    /// Where it was found, such as "GitHub CLI" or "Environment".
    pub source: String,
    pub account: String,
    pub connected: bool,
    /// A key listed on its own rather than an account sign-in.
    #[serde(default)]
    pub key: bool,
}

pub enum Found {
    Material(Box<AuthMaterial>),
    SignIn(BTreeMap<String, String>),
    Key(SecretValue),
}

/// The ID a source's discoveries start with.
pub fn source_id(s: &Source) -> String {
    match &s.from {
        SourceKind::Env { var } => format!("env:{var}"),
        SourceKind::GhCli => "gh".into(),
        SourceKind::AwsEnv => "env:AWS".into(),
        SourceKind::AwsProfiles => "aws_profile".into(),
        SourceKind::AwsSso => "aws_sso".into(),
        _ => s.id.clone(),
    }
}

/// Finds the manifest and source a discovery ID came from.
fn resolve<'a>(manifests: &'a [Manifest], id: &str) -> Option<(&'a Manifest, &'a Source, String)> {
    let (base, name) = match id.split_once(':') {
        Some((k @ ("gh" | "aws_profile" | "aws_sso"), name)) => (k.to_owned(), name.to_owned()),
        _ => (id.to_owned(), String::new()),
    };
    manifests.iter().find_map(|m| {
        m.sources
            .iter()
            .find(|s| source_id(s) == base)
            .map(|s| (m, s, name.clone()))
    })
}

/// Names an account's sign-in key after where it came from.
pub fn sign_in_name(manifests: &[Manifest], source: Option<&str>) -> String {
    match source {
        None => "Browser sign-in".into(),
        Some("token") => "Pasted token".into(),
        Some("getkeys") => "GetKeys token".into(),
        Some("env:AWS") => "Environment keys".into(),
        Some(s) if s.starts_with("env:") => format!("${}", &s[4..]),
        Some(s) if s.starts_with("aws_profile:") => format!("{} profile", &s[12..]),
        Some(s) => resolve(manifests, s)
            .map(|(m, src, _)| label(m, src))
            .unwrap_or_else(|| "Local sign-in".into()),
    }
}

fn label(m: &Manifest, s: &Source) -> String {
    if !s.label.is_empty() {
        return s.label.clone();
    }
    match &s.from {
        SourceKind::Env { var } => format!("${var}"),
        SourceKind::GhCli => "GitHub CLI".into(),
        SourceKind::AwsEnv => "Environment".into(),
        SourceKind::AwsProfiles => "AWS CLI profile".into(),
        SourceKind::AwsSso => "IAM Identity Center".into(),
        _ => format!("{} CLI", m.name),
    }
}

/// The default region configured for a local AWS source.
pub fn aws_region(source: Option<&str>) -> Option<String> {
    let (kind, name) = source?.split_once(':')?;
    match kind {
        "env" if name == "AWS" => env("AWS_REGION").or_else(|| env("AWS_DEFAULT_REGION")),
        "aws_profile" => {
            let section = if name == "default" {
                "default".to_owned()
            } else {
                format!("profile {name}")
            };
            ini(&aws_file("AWS_CONFIG_FILE", "config")?)
                .into_iter()
                .find(|(s, _)| *s == section)
                .and_then(|(_, keys)| keys.get("region").cloned())
                .filter(|r| !r.is_empty())
        }
        _ => None,
    }
}
/// Shortens ARN identities such as `arn:aws:iam::123456789012:user/alice` to `alice (123456789012)`.
pub fn friendly_name(name: &str) -> String {
    let parts: Vec<&str> = name.splitn(6, ':').collect();
    if parts.len() != 6 || parts[0] != "arn" {
        return name.to_owned();
    }
    let resource: Vec<&str> = parts[5].split('/').collect();
    let short = match resource.as_slice() {
        ["assumed-role", role, ..] => role,
        [.., last] => last,
        [] => return name.to_owned(),
    };
    format!("{short} ({})", parts[4])
}

/// Everything the given providers' sources find on this device. Values are not kept.
pub fn scan(manifests: &[Manifest]) -> Vec<Discovery> {
    let mut found: Vec<Discovery> = Vec::new();
    for m in manifests {
        for s in &m.sources {
            let base = source_id(s);
            let mut here = Vec::new();
            let mut add = |id: String, account: &str| {
                here.push(Discovery {
                    id,
                    plugin_id: m.id.clone(),
                    source: label(m, s),
                    account: account.into(),
                    connected: false,
                    key: s.role == SourceRole::Key,
                })
            };
            match &s.from {
                SourceKind::GhCli => {
                    for login in gh_accounts() {
                        if login.is_empty() {
                            add("gh".into(), "");
                        } else {
                            add(format!("gh:{login}"), &login);
                        }
                    }
                }
                SourceKind::AwsEnv => {
                    if env("AWS_ACCESS_KEY_ID").is_some() && env("AWS_SECRET_ACCESS_KEY").is_some()
                    {
                        add(base, "AWS_ACCESS_KEY_ID");
                    }
                }
                SourceKind::AwsProfiles => {
                    for (profile, _) in aws_static_profiles() {
                        add(format!("aws_profile:{profile}"), &profile);
                    }
                }
                SourceKind::AwsSso => {
                    let mut start_urls = Vec::new();
                    for (profile, sso) in aws_sso_profiles() {
                        if !start_urls.contains(&sso["start_url"]) {
                            start_urls.push(sso["start_url"].clone());
                            add(format!("aws_sso:{profile}"), &profile);
                        }
                    }
                }
                SourceKind::Env { var } => {
                    if env(var).is_some() {
                        add(base, var);
                    }
                }
                SourceKind::Keyring { hint, .. } if !cfg!(windows) => {
                    // Reading the macOS/Linux keychain prompts, so only a hint counts while scanning.
                    if !hint.is_empty() && expand(hint).is_some_and(|p| p.exists()) {
                        add(base, "");
                    }
                }
                _ => {
                    // Sources sharing a label are one place, such as a CLI's file and keychain.
                    let label = label(m, s);
                    let seen = found
                        .iter()
                        .any(|d| d.plugin_id == m.id && d.source == label && d.account.is_empty());
                    if !seen && read(s).is_ok_and(|(v, _)| !v.is_empty()) {
                        add(base, "");
                    }
                }
            }
            found.extend(here);
        }
    }
    found
}

/// Reads what a discovery found.
pub fn load(manifests: &[Manifest], id: &str) -> Result<(String, Found)> {
    let (m, s, name) = resolve(manifests, id).context("Unknown local source")?;
    let bearer = |token: String, expires_at| {
        Found::Material(Box::new(AuthMaterial::Bearer {
            token,
            refresh_token: None,
            expires_at,
            broker_registration: None,
        }))
    };
    let found = match &s.from {
        SourceKind::GhCli => {
            let mut args = vec!["auth", "token", "--hostname", "github.com"];
            if !name.is_empty() {
                ensure!(valid_login(&name), "Unknown GitHub CLI account");
                args.extend(["--user", &name]);
            }
            let token =
                run("gh", &args).context("GitHub CLI is not signed in. Run `gh auth login`.")?;
            bearer(token, None)
        }
        SourceKind::AwsEnv => Found::Material(Box::new(AuthMaterial::Aws {
            access_key_id: env("AWS_ACCESS_KEY_ID").context("AWS_ACCESS_KEY_ID is not set")?,
            secret_access_key: env("AWS_SECRET_ACCESS_KEY")
                .context("AWS_SECRET_ACCESS_KEY is not set")?,
            session_token: env("AWS_SESSION_TOKEN"),
            expires_at: None,
            sso: None,
        })),
        SourceKind::AwsSso => {
            let (_, config) = aws_sso_profiles()
                .into_iter()
                .find(|(p, _)| *p == name)
                .context("This AWS profile no longer uses IAM Identity Center")?;
            Found::SignIn(config)
        }
        SourceKind::AwsProfiles => {
            let (_, keys) = aws_static_profiles()
                .into_iter()
                .find(|(p, _)| *p == name)
                .context("This AWS profile no longer has access keys")?;
            Found::Material(Box::new(AuthMaterial::Aws {
                access_key_id: keys["aws_access_key_id"].clone(),
                secret_access_key: keys["aws_secret_access_key"].clone(),
                session_token: keys.get("aws_session_token").cloned(),
                expires_at: None,
                sso: None,
            }))
        }
        _ => {
            let (value, expires) = m
                .sources
                .iter()
                .filter(|o| o.id != s.id && label(m, o) == label(m, s))
                .fold(read(s), |r, other| {
                    r.or_else(|e| read(other).map_err(|_| e))
                })?;
            ensure!(!value.is_empty(), "{} has no key any more", label(m, s));
            if s.role == SourceRole::Key {
                Found::Key(SecretValue {
                    fields: BTreeMap::from([(s.field.clone(), value)]),
                })
            } else {
                bearer(value, expires)
            }
        }
    };
    Ok((m.id.clone(), found))
}

/// Reads a generic source: its value and, for sessions, when it expires.
fn read(s: &Source) -> Result<(String, Option<i64>)> {
    let missing = || {
        format!(
            "{} isn't signed in",
            if s.label.is_empty() { &s.id } else { &s.label }
        )
    };
    match &s.from {
        SourceKind::Env { var } => Ok((
            env(var).with_context(|| format!("{var} is no longer set"))?,
            None,
        )),
        SourceKind::Command { argv } => {
            let (program, args) = argv.split_first().context("Empty command")?;
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            Ok((run(program, &args).with_context(missing)?, None))
        }
        SourceKind::Keyring {
            service,
            user,
            target,
            go_keyring,
            ..
        } => {
            #[cfg(windows)]
            let entry = keyring::Entry::new_with_target(
                if target.is_empty() { service } else { target },
                service,
                user,
            );
            #[cfg(not(windows))]
            let entry = {
                let _ = target;
                keyring::Entry::new(service, user)
            };
            let bytes = entry?.get_secret().ok().with_context(missing)?;
            let value = if *go_keyring {
                decode_go_keyring(&bytes)
            } else {
                String::from_utf8(bytes).ok().map(|s| s.trim().to_owned())
            };
            Ok((value.filter(|v| !v.is_empty()).with_context(missing)?, None))
        }
        SourceKind::File {
            paths,
            format,
            pointer,
            expires_pointer,
            renew,
        } => {
            let read_file = || -> Result<(String, Option<i64>)> {
                let path = newest(paths).with_context(missing)?;
                let text = std::fs::read_to_string(path)?;
                let (value, expires) = match format {
                    FileFormat::Text => (Some(text.trim().to_owned()), None),
                    FileFormat::Ini => (ini_value(&text, pointer), None),
                    FileFormat::Json | FileFormat::Toml => {
                        let v: Value = if *format == FileFormat::Json {
                            serde_json::from_str(&text)?
                        } else {
                            serde_json::to_value(toml::from_str::<toml::Value>(&text)?)?
                        };
                        (
                            v.pointer(pointer)
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            (!expires_pointer.is_empty())
                                .then(|| v.pointer(expires_pointer).and_then(seconds))
                                .flatten(),
                        )
                    }
                };
                Ok((
                    value.filter(|v| !v.is_empty()).with_context(missing)?,
                    expires,
                ))
            };
            let (mut value, mut expires) = read_file()?;
            if expires.is_some_and(|e| e <= now() + 60)
                && let Some((program, args)) = renew.split_first()
            {
                // Any call renews the CLI's own session.
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                let _ = run(program, &args);
                (value, expires) = read_file()?;
            }
            ensure!(
                expires.is_none_or(|e| e > now()),
                "The {} session expired. Sign in to it again.",
                if s.label.is_empty() { "CLI" } else { &s.label }
            );
            Ok((value, expires))
        }
        _ => bail!("This source is read by its own reader"),
    }
}

/// Seconds since the epoch, from seconds, milliseconds, or an RFC 3339 date.
fn seconds(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .map(|n| if n > 100_000_000_000 { n / 1000 } else { n }),
        Value::String(s) => {
            time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                .ok()
                .map(|t| t.unix_timestamp())
        }
        _ => None,
    }
}

fn ini_value(text: &str, pointer: &str) -> Option<String> {
    let (section, key) = pointer.rsplit_once('.').unwrap_or(("", pointer));
    if section.is_empty() {
        return text.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim().eq_ignore_ascii_case(key)).then(|| v.trim().to_owned())
        });
    }
    ini(text)
        .into_iter()
        .find(|(s, _)| s == section)
        .and_then(|(_, keys)| keys.get(&key.to_lowercase()).cloned())
}

/// Expands `~`, `{home}`, `{config}`, `{data}`, `{local_data}` and `{env:NAME}` at the start of a
/// path. A `*` in the last part matches any file name.
pub fn expand(path: &str) -> Option<PathBuf> {
    let dirs = directories::BaseDirs::new()?;
    let (root, rest) = if let Some(rest) = path.strip_prefix("~/") {
        (dirs.home_dir().to_owned(), rest)
    } else {
        let (name, rest) = path.strip_prefix('{')?.split_once('}')?;
        let rest = rest.trim_start_matches(['/', '\\']);
        let root = match name {
            "home" => dirs.home_dir().to_owned(),
            "config" => dirs.config_dir().to_owned(),
            "data" => dirs.data_dir().to_owned(),
            "local_data" => dirs.data_local_dir().to_owned(),
            n => PathBuf::from(env(n.strip_prefix("env:")?)?),
        };
        (root, rest)
    };
    Some(
        rest.split(['/', '\\'])
            .filter(|p| !p.is_empty())
            .fold(root, |p, part| p.join(part)),
    )
}

/// The most recently changed file among `paths`.
fn newest(paths: &[String]) -> Option<PathBuf> {
    let mut files = Vec::new();
    for path in paths.iter().filter_map(|p| expand(p)) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let Some((prefix, suffix)) = name.split_once('*') else {
            files.push(path);
            continue;
        };
        let (prefix, suffix) = (prefix.to_owned(), suffix.to_owned());
        let Some(Ok(entries)) = path.parent().map(std::fs::read_dir) else {
            continue;
        };
        files.extend(
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|f| {
                    f.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                        n.len() >= prefix.len() + suffix.len()
                            && n.starts_with(&prefix)
                            && n.ends_with(&suffix)
                    })
                }),
        );
    }
    files
        .into_iter()
        .filter(|p| p.is_file())
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, p)| p)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn home() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_owned())
}

/// Runs a CLI without a console window; returns its stdout even when it exits non-zero.
fn exec(program: &str, args: &[&str]) -> Option<(bool, String)> {
    let spawn = |mut command: Command| {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        command
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
    };
    let mut direct = Command::new(program);
    direct.args(args);
    let output = match spawn(direct) {
        // npm installs CLIs as .cmd shims, which only cmd.exe resolves.
        Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::NotFound => {
            let mut shim = Command::new("cmd");
            shim.args(["/D", "/C", program]).args(args);
            spawn(shim)
        }
        other => other,
    }
    .ok()?;
    Some((
        output.status.success(),
        String::from_utf8(output.stdout).ok()?.trim().to_owned(),
    ))
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    exec(program, args).and_then(|(ok, text)| (ok && !text.is_empty()).then_some(text))
}

fn gh_accounts() -> Vec<String> {
    // `gh auth status` exits non-zero when any account needs attention; its JSON is still valid.
    if let Some((_, json)) = exec("gh", &["auth", "status", "--json", "hosts"])
        && let Ok(v) = serde_json::from_str::<Value>(&json)
    {
        return v["hosts"]["github.com"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| a["state"] == "success")
            .filter_map(|a| a["login"].as_str().map(str::to_owned))
            .filter(|login| valid_login(login))
            .collect();
    }
    run("gh", &["auth", "token"])
        .map(|_| vec![String::new()])
        .unwrap_or_default()
}

fn valid_login(login: &str) -> bool {
    !login.is_empty()
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
/// The Supabase CLI stores its token with go-keyring: UTF-8 bytes, sometimes prefixed and encoded.
pub fn decode_go_keyring(bytes: &[u8]) -> Option<String> {
    let utf16 = bytes.len() >= 2
        && bytes.len().is_multiple_of(2)
        && bytes.iter().skip(1).step_by(2).all(|b| *b == 0);
    let text = if utf16 {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16(&units).ok()?
    } else {
        String::from_utf8(bytes.to_vec()).ok()?
    };
    let text = text.trim_matches(char::from(0)).trim();
    let decoded = if let Some(b64) = text.strip_prefix("go-keyring-base64:") {
        String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b64).ok()?).ok()?
    } else if let Some(h) = text.strip_prefix("go-keyring-encoded:") {
        String::from_utf8(hex::decode(h).ok()?).ok()?
    } else {
        text.to_owned()
    };
    let decoded = decoded.trim().to_owned();
    (!decoded.is_empty()).then_some(decoded)
}

fn aws_file(var: &str, name: &str) -> Option<String> {
    let path = env(var)
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".aws").join(name)))?;
    std::fs::read_to_string(path).ok()
}

fn ini(text: &str) -> Vec<(String, BTreeMap<String, String>)> {
    let mut sections: Vec<(String, BTreeMap<String, String>)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((name.trim().into(), BTreeMap::new()));
        } else if let (Some((key, value)), Some((_, section))) =
            (line.split_once('='), sections.last_mut())
        {
            section.insert(key.trim().to_lowercase(), value.trim().to_owned());
        }
    }
    sections
}

fn aws_static_profiles() -> Vec<(String, BTreeMap<String, String>)> {
    ini(&aws_file("AWS_SHARED_CREDENTIALS_FILE", "credentials").unwrap_or_default())
        .into_iter()
        .filter(|(_, keys)| {
            ["aws_access_key_id", "aws_secret_access_key"]
                .iter()
                .all(|k| keys.get(*k).is_some_and(|v| !v.is_empty()))
        })
        .collect()
}

fn aws_sso_profiles() -> Vec<(String, BTreeMap<String, String>)> {
    let sections = ini(&aws_file("AWS_CONFIG_FILE", "config").unwrap_or_default());
    let sessions: BTreeMap<_, _> = sections
        .iter()
        .filter_map(|(name, keys)| Some((name.strip_prefix("sso-session ")?.trim(), keys)))
        .collect();
    sections
        .iter()
        .filter_map(|(name, keys)| {
            let profile = match name.as_str() {
                "default" => "default",
                n => n.strip_prefix("profile ")?.trim(),
            };
            let source = keys
                .get("sso_session")
                .and_then(|s| sessions.get(s.as_str()).copied())
                .unwrap_or(keys);
            let start_url = source.get("sso_start_url")?;
            let region = source.get("sso_region")?;
            Some((
                profile.to_owned(),
                BTreeMap::from([
                    ("start_url".to_owned(), start_url.clone()),
                    ("region".to_owned(), region.clone()),
                ]),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortens_arn_identities() {
        assert_eq!(
            friendly_name("arn:aws:iam::123456789012:user/ops/alice"),
            "alice (123456789012)"
        );
        assert_eq!(
            friendly_name("arn:aws:sts::123456789012:assumed-role/Admin/session"),
            "Admin (123456789012)"
        );
        assert_eq!(friendly_name("octocat"), "octocat");
    }
    #[test]
    fn reads_sso_sessions_and_legacy_profiles() {
        let sections = ini(
            "[profile a]\nsso_session = corp\n[sso-session corp]\nsso_start_url = https://corp.awsapps.com/start\nsso_region = eu-west-2\n# note\n[default]\nregion = us-east-1\n",
        );
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[1].1["sso_region"], "eu-west-2");
    }
    #[test]
    fn decodes_go_keyring_values() {
        assert_eq!(decode_go_keyring(b"sbp_abc").as_deref(), Some("sbp_abc"));
        assert_eq!(
            decode_go_keyring(b"go-keyring-base64:c2JwX2FiYw==").as_deref(),
            Some("sbp_abc")
        );
        let utf16: Vec<u8> = "sbp_abc"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(decode_go_keyring(&utf16).as_deref(), Some("sbp_abc"));
    }
    fn manifest(sources: serde_json::Value) -> Manifest {
        serde_json::from_value(serde_json::json!({
            "id": "test.acme", "name": "Acme", "publisher": "t", "version": "1", "description": "",
            "homepage": "https://acme.dev", "kind": "declarative", "hosts": ["api.acme.dev"],
            "credential_fields": [{"key": "token", "label": "API key", "env": "ACME_API_KEY"}],
            "sources": sources,
        }))
        .unwrap()
    }
    #[test]
    fn evaluates_manifest_sources() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().display().to_string();
        std::fs::write(
            dir.path().join("auth.json"),
            r#"{"auth":{"key":"acme_json"},"exp":1}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("creds.ini"),
            "[default]\napi_key = acme_ini\n",
        )
        .unwrap();
        std::fs::create_dir(dir.path().join("profiles")).unwrap();
        std::fs::write(
            dir.path().join("profiles").join("work.key"),
            "  acme_text\n",
        )
        .unwrap();
        // SAFETY: the variable names are unique to this test.
        unsafe {
            std::env::set_var("GETKEYS_TEST_ROOT", &root);
            std::env::set_var("GETKEYS_TEST_ACME_KEY", "acme_env");
        }
        let m = manifest(serde_json::json!([
            {"kind": "env", "var": "GETKEYS_TEST_ACME_KEY", "role": "key"},
            {"id": "acme_ini", "label": "Acme CLI", "kind": "file", "role": "key",
             "paths": ["{env:GETKEYS_TEST_ROOT}/creds.ini"], "format": "ini", "pointer": "default.api_key"},
            {"id": "acme_text", "label": "Acme profiles", "kind": "file", "role": "key",
             "paths": ["{env:GETKEYS_TEST_ROOT}/profiles/*.key"]},
            {"id": "acme_json", "label": "Acme session", "kind": "file",
             "paths": ["{env:GETKEYS_TEST_ROOT}/missing.json", "{env:GETKEYS_TEST_ROOT}/auth.json"],
             "format": "json", "pointer": "/auth/key", "expires_pointer": "/exp"},
            {"id": "acme_absent", "kind": "env", "var": "GETKEYS_TEST_UNSET"}
        ]));
        let ms = [m];
        let found = scan(&ms);
        let ids: Vec<&str> = found.iter().map(|d| d.id.as_str()).collect();
        // The JSON session expired long ago and has no renew command, so it isn't offered.
        assert_eq!(ids, ["env:GETKEYS_TEST_ACME_KEY", "acme_ini", "acme_text"]);
        assert!(found.iter().all(|d| d.key));
        let key = |id: &str| match load(&ms, id).unwrap().1 {
            Found::Key(v) => v.fields["token"].clone(),
            _ => panic!("expected a key"),
        };
        assert_eq!(key("env:GETKEYS_TEST_ACME_KEY"), "acme_env");
        assert_eq!(key("acme_ini"), "acme_ini");
        assert_eq!(key("acme_text"), "acme_text");
        assert!(load(&ms, "acme_json").is_err_and(|e| e.to_string().contains("expired")));
        assert_eq!(sign_in_name(&ms, Some("acme_ini")), "Acme CLI");
        assert_eq!(
            sign_in_name(&ms, Some("env:GETKEYS_TEST_ACME_KEY")),
            "$GETKEYS_TEST_ACME_KEY"
        );
    }
    #[test]
    fn sources_sharing_a_label_act_as_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.txt"), "second").unwrap();
        // SAFETY: the variable name is unique to this test.
        unsafe { std::env::set_var("GETKEYS_TEST_SHARED", dir.path()) };
        let ms = [manifest(serde_json::json!([
            {"id": "first", "label": "Acme CLI", "kind": "file", "paths": ["{env:GETKEYS_TEST_SHARED}/a.txt"]},
            {"id": "second", "label": "Acme CLI", "kind": "file", "paths": ["{env:GETKEYS_TEST_SHARED}/b.txt"]}
        ]))];
        assert_eq!(scan(&ms).len(), 1);
        // A sign-in saved from the first place still loads after the CLI moved its token.
        assert!(matches!(load(&ms, "first").unwrap().1, Found::Material(_)));
    }
}
