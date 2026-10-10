//! Work shared by the commands and the MCP server. Nothing here writes to stdout.
use crate::{
    client::Client,
    keys::{self, Key},
};
use anyhow::{Result, ensure};
use getkeys_core::{
    api::{OperationKind, Resource},
    commands::Command,
    discover::Discovery,
    model::{ConnectionState, EnvLine, SIGN_IN, Snapshot},
};
use regex::{Captures, Regex};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::LazyLock,
};

static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{\{\s*(getkeys://[^\s{}]+)\s*\}\}|getkeys://[\w.~-]+/[\w.~-]+(?:/[\w.~-]+)?")
        .expect("valid pattern")
});

/// Values the program receives, and which of them are secret.
#[derive(Default)]
pub struct Env {
    pub vars: BTreeMap<String, String>,
    pub secrets: BTreeSet<String>,
}

fn read(
    client: &mut Client,
    lines: Vec<EnvLine>,
    via: Option<&str>,
) -> Result<BTreeMap<String, String>> {
    if lines.is_empty() {
        return Ok(BTreeMap::new());
    }
    client.get(Command::Resolve {
        lines,
        via: via.map(Into::into),
    })
}

fn secret(key: &Key, field: &str) -> bool {
    key.c.fields.iter().any(|f| f.key == field && f.secret)
}

/// The value one reference or ID names, and whether it is secret.
pub fn value(
    client: &mut Client,
    reference: &str,
    field: Option<&str>,
    via: Option<&str>,
) -> Result<(String, bool)> {
    let snapshot = client.snapshot()?;
    let keys = keys::keys(&snapshot);
    let (key, named) = keys::find(&keys, reference)?;
    let field = key.field(field.or(named.as_deref()))?;
    let line = EnvLine {
        credential_id: key.c.id.clone(),
        field: field.clone(),
        variable: "VALUE".into(),
    };
    let secret = secret(key, &field);
    let value = read(client, vec![line], via)?
        .remove("VALUE")
        .unwrap_or_default();
    Ok((value, secret))
}

/// Replaces each `getkeys://` reference (bare or in `{{ }}`) in `texts` with its value.
pub fn substitute(
    client: &mut Client,
    snapshot: &Snapshot,
    texts: &[String],
    via: Option<&str>,
    env: &mut Env,
) -> Result<Vec<String>> {
    let keys = keys::keys(snapshot);
    let reference = |c: &Captures| {
        c.get(1)
            .unwrap_or_else(|| c.get(0).expect("match"))
            .as_str()
            .to_owned()
    };
    let mut found: BTreeMap<String, (String, String)> = BTreeMap::new();
    for text in texts {
        for c in REFERENCE.captures_iter(text) {
            let r = reference(&c);
            if let std::collections::btree_map::Entry::Vacant(entry) = found.entry(r) {
                let (key, field) = keys::find(&keys, entry.key())?;
                entry.insert((key.c.id.clone(), key.field(field.as_deref())?));
            }
        }
    }
    let pairs: Vec<_> = found.iter().collect();
    let lines = pairs
        .iter()
        .enumerate()
        .map(|(i, (_, (id, field)))| EnvLine {
            credential_id: id.clone(),
            field: field.clone(),
            variable: format!("V{i}"),
        })
        .collect();
    let mut values = read(client, lines, via)?;
    let mut resolved = BTreeMap::new();
    for (i, (r, (id, field))) in pairs.iter().enumerate() {
        let value = values.remove(&format!("V{i}")).unwrap_or_default();
        if keys.iter().any(|k| k.c.id == *id && secret(k, field)) {
            env.secrets.insert(value.clone());
        }
        resolved.insert(r.to_string(), value);
    }
    Ok(texts
        .iter()
        .map(|t| {
            REFERENCE
                .replace_all(t, |c: &Captures| resolved[&reference(c)].clone())
                .into_owned()
        })
        .collect())
}

/// `KEY=value` lines of a `.env` file.
pub fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            if line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = value.trim();
            let value = if let Some(v) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                v.replace("\\n", "\n")
                    .replace("\\\"", "\"")
                    .replace("\\\\", "\\")
            } else if let Some(v) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
                v.to_owned()
            } else {
                value
                    .split(" #")
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            };
            Some((key.trim().to_owned(), value))
        })
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

/// What a program started with keys receives: references in this process's variables and in
/// `env_file` resolved, then the chosen keys under their export names.
pub fn environment(
    client: &mut Client,
    groups: &[String],
    references: &[String],
    env_file: Option<&Path>,
    via: Option<&str>,
) -> Result<Env> {
    let snapshot = client.snapshot()?;
    let mut env = Env::default();
    let mut pending: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .filter(|(_, v)| v.contains(keys::SCHEME))
        .collect();
    if let Some(path) = env_file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Unable to read {}: {e}", path.display()))?;
        pending.extend(parse_dotenv(&text));
    }
    let texts: Vec<String> = pending.iter().map(|(_, v)| v.clone()).collect();
    let values = substitute(client, &snapshot, &texts, via, &mut env)?;
    for ((name, _), value) in pending.into_iter().zip(values) {
        env.vars.insert(name, value);
    }
    let keys = keys::keys(&snapshot);
    let chosen = keys::select(&keys, groups, references)?;
    if !chosen.is_empty() {
        let lines = keys::env_lines(&chosen);
        ensure!(
            !lines.is_empty(),
            "The chosen keys have no values to export"
        );
        let secret: BTreeSet<String> = lines
            .iter()
            .filter(|l| {
                keys.iter()
                    .any(|k| k.c.id == l.credential_id && secret(k, &l.field))
            })
            .map(|l| l.variable.clone())
            .collect();
        for (name, value) in read(client, lines, via)? {
            if secret.contains(&name) {
                env.secrets.insert(value.clone());
            }
            env.vars.insert(name, value);
        }
    }
    Ok(env)
}

/// The chosen keys as `.env` text.
pub fn dotenv(
    client: &mut Client,
    groups: &[String],
    references: &[String],
    via: Option<&str>,
) -> Result<String> {
    let snapshot = client.snapshot()?;
    let keys = keys::keys(&snapshot);
    let chosen = if groups.is_empty() && references.is_empty() {
        keys.iter().map(|k| (k, None)).collect()
    } else {
        keys::select(&keys, groups, references)?
    };
    client.get(Command::ExportEnv {
        lines: keys::env_lines(&chosen),
        via: via.map(Into::into),
    })
}

/// The program to start: on Windows a bare name is looked up with `PATHEXT`, so `npm` finds
/// `npm.cmd`.
pub fn program(name: &str) -> PathBuf {
    let path = PathBuf::from(name);
    if !cfg!(windows) || path.extension().is_some() || path.components().count() > 1 {
        return path;
    }
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let dirs = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&dirs)
        .flat_map(|dir| {
            exts.split(';')
                .map(move |ext| dir.join(format!("{name}{ext}")))
        })
        .find(|p| p.is_file())
        .unwrap_or(path)
}

/// Replaces secret values in program output.
pub fn redact(text: &str, secrets: &BTreeSet<String>) -> String {
    let mut secrets: Vec<&String> = secrets.iter().filter(|s| s.len() >= 4).collect();
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    secrets
        .into_iter()
        .fold(text.to_owned(), |t, s| t.replace(s.as_str(), "[redacted]"))
}

/// Looks for new sign-ins and keys on this device, lists every account's keys, and checks the
/// keys no listing vouches for. With a group, only the accounts and keys in it.
pub fn refresh(
    client: &mut Client,
    group: Option<&str>,
    progress: &mut dyn FnMut(&str),
) -> Result<String> {
    let mut failed: Vec<String> = Vec::new();
    let before: BTreeSet<String> = keys::keys(&client.snapshot()?)
        .iter()
        .map(|k| k.c.id.clone())
        .collect();
    if group.is_none() {
        progress("Looking for sign-ins and keys on this device");
        let found: Vec<Discovery> = client.get(Command::ScanSources { plugin_id: None })?;
        for d in found {
            let name = if d.account.is_empty() {
                &d.source
            } else {
                &d.account
            };
            progress(&format!("Saving {name}"));
            if let Err(e) = client.call(Command::ConnectLocal {
                source_id: d.id.clone(),
                connection_id: None,
            }) {
                failed.push(format!("{name}: {e:#}"));
            }
        }
    }
    let snapshot = client.snapshot()?;
    let keys = keys::keys(&snapshot);
    let in_scope = |id: &str| match group {
        Some(g) => keys.iter().any(|k| k.c.id == id && k.in_group(g)),
        None => true,
    };
    if let Some(g) = group {
        ensure!(
            keys.iter().any(|k| k.in_group(g)),
            "No keys are in the group {g}"
        );
    }
    for c in snapshot.connections.iter().filter(|c| {
        c.state != ConnectionState::Disconnected
            && (group.is_none()
                || snapshot
                    .credentials
                    .iter()
                    .any(|k| k.connection_id == c.id && in_scope(&k.id)))
    }) {
        let Some(m) = snapshot
            .plugins
            .iter()
            .find(|p| p.manifest.id == c.plugin_id)
        else {
            continue;
        };
        let m = &m.manifest;
        progress(&format!("Listing keys in {} · {}", m.name, c.name));
        let lists = m.operations.contains(&OperationKind::ListCredentials);
        let places: Vec<String> = if m.account_credentials {
            vec![String::new()]
        } else if lists {
            match client.get::<Vec<Resource>>(Command::Resources {
                connection_id: c.id.clone(),
            }) {
                Ok(r) => r.into_iter().map(|r| r.id).collect(),
                Err(e) => {
                    failed.push(format!("{} · {}: {e:#}", m.name, c.name));
                    continue;
                }
            }
        } else {
            Vec::new()
        };
        for resource in places {
            if let Err(e) = client.call(Command::Sync {
                connection_id: c.id.clone(),
                resource,
            }) {
                failed.push(format!("{} · {}: {e:#}", m.name, c.name));
            }
        }
        for k in snapshot.credentials.iter().filter(|k| {
            k.connection_id == c.id
                && k.credential_type != SIGN_IN
                && !k.retired
                && k.value_available
                && (!lists || k.provider_id.starts_with("import-"))
        }) {
            let _ = client.call(Command::Check {
                credential_id: k.id.clone(),
            });
        }
    }
    for k in snapshot.credentials.iter().filter(|k| {
        !k.plugin_id.is_empty()
            && k.connection_id.is_empty()
            && !k.retired
            && k.value_available
            && in_scope(&k.id)
    }) {
        progress(&format!("Checking {}", k.name));
        if let Err(e) = client.call(Command::Check {
            credential_id: k.id.clone(),
        }) {
            failed.push(format!("{}: {e:#}", k.name));
        }
    }
    let after = keys::keys(&client.snapshot()?)
        .iter()
        .filter(|k| !before.contains(&k.c.id))
        .count();
    let mut summary = match after {
        0 => "Keys are up to date".to_owned(),
        1 => "Found 1 key".to_owned(),
        n => format!("Found {n} keys"),
    };
    for f in failed {
        summary.push_str(&format!("\nFailed: {f}"));
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_dotenv_files() {
        let text = "# comment\nexport A=1\nB=\"x\\ny\"\nC='raw value'\nD=plain # note\n=skip\n";
        assert_eq!(
            parse_dotenv(text),
            [
                ("A".into(), "1".into()),
                ("B".into(), "x\ny".into()),
                ("C".into(), "raw value".into()),
                ("D".into(), "plain".into()),
            ]
        );
    }

    #[test]
    fn finds_references() {
        let found: Vec<&str> = REFERENCE
            .find_iter("a={{ getkeys://custom/x }} b=getkeys://openai/prod/token c=getkeys://")
            .map(|m| m.as_str())
            .collect();
        assert_eq!(
            found,
            ["{{ getkeys://custom/x }}", "getkeys://openai/prod/token"]
        );
    }

    #[test]
    fn redacts_longest_secrets_first() {
        let secrets = BTreeSet::from(["abcd".into(), "abcdefgh".into(), "x".into()]);
        assert_eq!(
            redact("abcdefgh abcd x", &secrets),
            "[redacted] [redacted] x"
        );
    }
}
