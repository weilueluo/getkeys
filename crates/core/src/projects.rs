//! Keys in project `.env` files, inside folders the person chose to have scanned.
use crate::discover::Discovery;
use anyhow::{Context, Result};
use getkeys_plugin_api::{Manifest, SecretValue};
use regex::Regex;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const PREFIX: &str = "dotenv:";
const DEPTH: usize = 4;
const SKIP: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    "venv",
    ".venv",
    "__pycache__",
];

/// `.env`, `.env.local`, `.env.production`… but not the examples meant to be committed.
fn is_env_file(name: &str) -> bool {
    (name == ".env" || name.starts_with(".env."))
        && ![".example", ".sample", ".template", ".dist", ".defaults"]
            .iter()
            .any(|s| name.ends_with(s))
}

fn env_files(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() && is_env_file(&name) {
            out.push(entry.path());
        } else if kind.is_dir()
            && depth > 0
            && !name.starts_with('.')
            && !SKIP.contains(&name.as_str())
        {
            env_files(&entry.path(), depth - 1, out);
        }
    }
}

/// `NAME=value` pairs, with `export`, quotes and trailing comments handled the usual way.
pub fn parse(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (name, value) = line.split_once('=')?;
            let name = name.trim();
            if name.is_empty() || name.starts_with('#') || name.contains(char::is_whitespace) {
                return None;
            }
            let value = value.trim();
            let value = match value.chars().next() {
                Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or_default(),
                _ => value.split(" #").next().unwrap_or_default().trim(),
            };
            Some((name.to_owned(), value.to_owned()))
        })
        .collect()
}

fn patterns(manifests: &[Manifest]) -> Vec<(&Manifest, Regex)> {
    manifests
        .iter()
        .filter(|m| !m.key_pattern.is_empty())
        .filter_map(|m| Some((m, Regex::new(&m.key_pattern).ok()?)))
        .collect()
}

/// The provider whose key pattern the value matches, as the value its key fields expect.
fn identify<'a>(
    patterns: &[(&'a Manifest, Regex)],
    value: &str,
) -> Option<(&'a Manifest, SecretValue)> {
    let (m, _) = patterns.iter().find(|(_, p)| p.is_match(value))?;
    let field = m
        .credential_fields
        .iter()
        .find(|f| f.secret)
        .map_or("token", |f| f.key.as_str());
    Some((
        m,
        SecretValue {
            fields: BTreeMap::from([(field.to_owned(), value.to_owned())]),
        },
    ))
}

fn id(path: &Path, name: &str) -> String {
    format!("{PREFIX}{}#{name}", path.display())
}

fn label(folder: &Path, path: &Path) -> String {
    let folder_name = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let inner = path.strip_prefix(folder).unwrap_or(path);
    Path::new(&folder_name)
        .join(inner)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Every value in the folders' `.env` files that matches a provider's key pattern.
pub fn scan(folders: &[String], manifests: &[Manifest]) -> Vec<(Discovery, SecretValue)> {
    let patterns = patterns(manifests);
    let mut found = Vec::new();
    for folder in folders.iter().map(PathBuf::from) {
        let mut files = Vec::new();
        env_files(&folder, DEPTH, &mut files);
        files.sort();
        for path in files {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (name, value) in parse(&text) {
                if let Some((m, secret)) = identify(&patterns, &value) {
                    found.push((
                        Discovery {
                            id: id(&path, &name),
                            plugin_id: m.id.clone(),
                            source: label(&folder, &path),
                            account: name,
                            connected: false,
                            key: true,
                        },
                        secret,
                    ));
                }
            }
        }
    }
    found
}

pub fn is_project(source: &str) -> bool {
    source.starts_with(PREFIX)
}

/// Reads one scanned value again: its provider, name and value.
pub fn load(manifests: &[Manifest], source: &str) -> Result<(String, String, SecretValue)> {
    let (path, name) = source
        .strip_prefix(PREFIX)
        .and_then(|s| s.rsplit_once('#'))
        .context("Not a project key")?;
    let text = std::fs::read_to_string(path).with_context(|| format!("Unable to read {path}"))?;
    let value = parse(&text)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
        .with_context(|| format!("{name} is no longer in {path}"))?;
    let (m, secret) =
        identify(&patterns(manifests), &value).context("This value isn't a known provider key")?;
    let file = Path::new(path)
        .parent()
        .and_then(Path::file_name)
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok((m.id.clone(), format!("{name} · {file}"), secret))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotenv_lines() {
        let parsed = parse(
            "# comment\nexport A=1\nB = \"two words\" # note\nC='x#y'\nD=plain # trailing\n\nbad line\n",
        );
        assert_eq!(
            parsed,
            [
                ("A".into(), "1".into()),
                ("B".into(), "two words".into()),
                ("C".into(), "x#y".into()),
                ("D".into(), "plain".into()),
            ]
        );
    }

    #[test]
    fn finds_provider_keys_in_project_env_files() {
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "id": "test.acme", "name": "Acme", "publisher": "t", "version": "1", "description": "",
            "homepage": "https://acme.dev", "hosts": [], "key_pattern": "^acme_[a-z]{6}$",
            "credential_fields": [{"key": "api_key", "label": "API key"}]
        }))
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        std::fs::create_dir_all(app.join("node_modules")).unwrap();
        std::fs::write(app.join(".env"), "ACME_KEY=acme_abcdef\nOTHER=nope\n").unwrap();
        std::fs::write(app.join(".env.example"), "ACME_KEY=acme_zzzzzz\n").unwrap();
        std::fs::write(app.join("node_modules/.env"), "ACME_KEY=acme_yyyyyy\n").unwrap();
        let folders = [dir.path().display().to_string()];
        let found = scan(&folders, std::slice::from_ref(&manifest));
        assert_eq!(found.len(), 1);
        let (d, value) = &found[0];
        assert_eq!(d.plugin_id, "test.acme");
        assert_eq!(d.account, "ACME_KEY");
        assert!(d.source.ends_with("app/.env"));
        assert_eq!(value.fields["api_key"], "acme_abcdef");
        let (plugin, name, again) = load(std::slice::from_ref(&manifest), &d.id).unwrap();
        assert_eq!(
            (plugin.as_str(), name.as_str()),
            ("test.acme", "ACME_KEY · app")
        );
        assert_eq!(again.fields, value.fields);
    }
}
