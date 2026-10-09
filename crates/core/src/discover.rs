//! Finds provider sessions that already exist on this device, so connecting is one click:
//! CLI logins (gh, vercel, supabase), AWS profiles, and well-known environment variables.
use crate::{model::AuthMaterial, now};
use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, process::Command};
use ts_rs::TS;

const GITHUB: &str = "official.github";
const VERCEL: &str = "official.vercel";
const SUPABASE: &str = "official.supabase";
const AWS: &str = "official.aws";

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct Discovery {
    pub id: String,
    pub plugin_id: String,
    pub source: String,
    pub account: String,
    pub connected: bool,
}

pub enum Found {
    Material(Box<AuthMaterial>),
    SignIn(BTreeMap<String, String>),
}

/// Names an account's sign-in key after where it came from.
pub fn sign_in_name(source: Option<&str>) -> String {
    let (kind, name) = source.map_or(("", ""), |s| s.split_once(':').unwrap_or((s, "")));
    match kind {
        "gh" => "GitHub CLI".into(),
        "vercel" => "Vercel CLI".into(),
        "supabase" => "Supabase CLI".into(),
        "env" if name == "AWS" => "Environment keys".into(),
        "env" => format!("${name}"),
        "aws_profile" => format!("{name} profile"),
        "token" => "Pasted token".into(),
        "getkeys" => "GetKeys token".into(),
        _ => "Browser sign-in".into(),
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

pub fn scan() -> Vec<Discovery> {
    let mut found = Vec::new();
    let mut add = |id: String, plugin_id: &str, source: &str, account: &str| {
        found.push(Discovery {
            id,
            plugin_id: plugin_id.into(),
            source: source.into(),
            account: account.into(),
            connected: false,
        })
    };
    for login in gh_accounts() {
        let id = if login.is_empty() {
            "gh".into()
        } else {
            format!("gh:{login}")
        };
        add(id, GITHUB, "GitHub CLI", &login);
    }
    for (var, plugin, source) in ENV_TOKENS {
        if env(var).is_some() {
            add(format!("env:{var}"), plugin, source, var);
        }
    }
    if vercel_auth_file().is_some() {
        add("vercel".into(), VERCEL, "Vercel CLI", "");
    }
    if supabase_present() {
        add("supabase".into(), SUPABASE, "Supabase CLI", "");
    }
    if env("AWS_ACCESS_KEY_ID").is_some() && env("AWS_SECRET_ACCESS_KEY").is_some() {
        add("env:AWS".into(), AWS, "Environment", "AWS_ACCESS_KEY_ID");
    }
    let mut start_urls = Vec::new();
    for (profile, sso) in aws_sso_profiles() {
        if !start_urls.contains(&sso["start_url"]) {
            start_urls.push(sso["start_url"].clone());
            add(
                format!("aws_sso:{profile}"),
                AWS,
                "IAM Identity Center",
                &profile,
            );
        }
    }
    for (profile, _) in aws_static_profiles() {
        add(
            format!("aws_profile:{profile}"),
            AWS,
            "AWS CLI profile",
            &profile,
        );
    }
    found
}

pub fn load(id: &str) -> Result<(String, Found)> {
    let oauth = |token: String, expires_at| {
        Found::Material(Box::new(AuthMaterial::Bearer {
            token,
            refresh_token: None,
            expires_at,
            broker_registration: None,
        }))
    };
    let (kind, name) = id.split_once(':').unwrap_or((id, ""));
    Ok(match kind {
        "gh" => {
            let mut args = vec!["auth", "token", "--hostname", "github.com"];
            if !name.is_empty() {
                ensure!(valid_login(name), "Unknown GitHub CLI account");
                args.extend(["--user", name]);
            }
            let token =
                run("gh", &args).context("GitHub CLI is not signed in. Run `gh auth login`.")?;
            (GITHUB.into(), oauth(token, None))
        }
        "env" if name == "AWS" => (
            AWS.into(),
            Found::Material(Box::new(AuthMaterial::Aws {
                access_key_id: env("AWS_ACCESS_KEY_ID").context("AWS_ACCESS_KEY_ID is not set")?,
                secret_access_key: env("AWS_SECRET_ACCESS_KEY")
                    .context("AWS_SECRET_ACCESS_KEY is not set")?,
                session_token: env("AWS_SESSION_TOKEN"),
                expires_at: None,
                sso: None,
            })),
        ),
        "env" => {
            let (_, plugin, _) = ENV_TOKENS
                .iter()
                .find(|(var, ..)| *var == name)
                .context("Unknown environment source")?;
            let token = env(name).with_context(|| format!("{name} is no longer set"))?;
            ((*plugin).into(), oauth(token, None))
        }
        "vercel" => {
            let (mut token, mut expires) = vercel_token()?;
            if expires.is_some_and(|e| e <= now() + 60) {
                // Any CLI call renews the CLI's own session.
                let _ = run("vercel", &["whoami"]);
                (token, expires) = vercel_token()?;
            }
            ensure!(
                expires.is_none_or(|e| e > now()),
                "The Vercel CLI session expired. Run `vercel login`."
            );
            (VERCEL.into(), oauth(token, expires))
        }
        "supabase" => (
            SUPABASE.into(),
            oauth(
                supabase_token().context("Supabase CLI is not signed in. Run `supabase login`.")?,
                None,
            ),
        ),
        "aws_sso" => {
            let (_, config) = aws_sso_profiles()
                .into_iter()
                .find(|(p, _)| p == name)
                .context("This AWS profile no longer uses IAM Identity Center")?;
            (AWS.into(), Found::SignIn(config))
        }
        "aws_profile" => {
            let (_, keys) = aws_static_profiles()
                .into_iter()
                .find(|(p, _)| p == name)
                .context("This AWS profile no longer has access keys")?;
            (
                AWS.into(),
                Found::Material(Box::new(AuthMaterial::Aws {
                    access_key_id: keys["aws_access_key_id"].clone(),
                    secret_access_key: keys["aws_secret_access_key"].clone(),
                    session_token: keys.get("aws_session_token").cloned(),
                    expires_at: None,
                    sso: None,
                })),
            )
        }
        _ => bail!("Unknown local source"),
    })
}

const ENV_TOKENS: [(&str, &str, &str); 5] = [
    ("GH_TOKEN", GITHUB, "Environment"),
    ("GITHUB_TOKEN", GITHUB, "Environment"),
    ("VERCEL_TOKEN", VERCEL, "Environment"),
    ("SUPABASE_ACCESS_TOKEN", SUPABASE, "Environment"),
    ("SUPABASE_TOKEN", SUPABASE, "Environment"),
];

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
    let mut command = if cfg!(windows) && program == "vercel" {
        // npm installs the Vercel CLI as a .cmd shim, which only cmd.exe resolves.
        let mut c = Command::new("cmd");
        c.args(["/D", "/C", "vercel"]);
        c
    } else {
        Command::new(program)
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
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
/// The Vercel CLI keeps its session wherever xdg-app-paths points; take the newest candidate.
fn vercel_auth_file() -> Option<PathBuf> {
    let dirs = directories::BaseDirs::new()?;
    let name = "com.vercel.cli";
    let mut candidates = vec![
        dirs.data_dir().join("xdg.data").join(name),
        dirs.data_dir().join(name),
        dirs.data_dir().join(name).join("Data"),
        dirs.home_dir().join(".local").join("share").join(name),
        dirs.home_dir().join(".now"),
    ];
    if let Some(xdg) = env("XDG_DATA_HOME") {
        candidates.insert(0, PathBuf::from(xdg).join(name));
    }
    candidates
        .into_iter()
        .map(|d| d.join("auth.json"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, p)| p)
}

fn vercel_token() -> Result<(String, Option<i64>)> {
    let path = vercel_auth_file().context("Vercel CLI is not signed in. Run `vercel login`.")?;
    let v: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let token = v["token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .context("Vercel CLI is not signed in. Run `vercel login`.")?;
    Ok((token.into(), v["expiresAt"].as_i64()))
}
fn supabase_file() -> Option<PathBuf> {
    home().map(|h| h.join(".supabase").join("access-token"))
}

fn supabase_present() -> bool {
    if supabase_file().is_some_and(|p| p.is_file()) {
        return true;
    }
    // Reading the macOS/Linux keychain prompts the user, so only peek where it is silent.
    if cfg!(windows) {
        supabase_keyring().is_some()
    } else {
        home().is_some_and(|h| h.join(".supabase").is_dir())
    }
}

fn supabase_token() -> Option<String> {
    supabase_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(supabase_keyring)
}

fn supabase_keyring() -> Option<String> {
    #[cfg(windows)]
    let entry =
        keyring::Entry::new_with_target("Supabase CLI:supabase", "Supabase CLI", "supabase");
    #[cfg(not(windows))]
    let entry = keyring::Entry::new("Supabase CLI", "supabase");
    decode_go_keyring(&entry.ok()?.get_secret().ok()?)
}

/// The Supabase CLI stores its token with go-keyring: UTF-8 bytes, sometimes prefixed and encoded.
fn decode_go_keyring(bytes: &[u8]) -> Option<String> {
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
}
