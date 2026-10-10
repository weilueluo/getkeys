//! The keys GetKeys lists, how they're named in `getkeys://` references, and how they export.
use anyhow::{Context, Result, bail, ensure};
use getkeys_core::{
    api::OperationKind,
    app::CUSTOM,
    model::{ConnectionView, CredentialView, EnvLine, SIGN_IN, Snapshot},
};
use serde_json::{Value, json};

pub const SCHEME: &str = "getkeys://";

/// One key as the Keys page lists it.
pub struct Key<'a> {
    pub c: &'a CredentialView,
    /// Provider name, or `Custom`.
    pub provider: String,
    /// `None` for keys without a provider.
    pub plugin_id: Option<String>,
    pub account: Option<String>,
    /// Where the key lives; tells keys with the same variable apart in a `.env` file.
    pub place: String,
    /// Listed by the provider, which never shows its secret again.
    pub secretless: bool,
    pub reference: String,
}

/// Whether Keys lists a key: every live key except management keys that weren't found here
/// (mirrors `listed` in the desktop app).
fn listed(c: &CredentialView, connections: &[ConnectionView]) -> bool {
    !c.retired
        && (c.credential_type != SIGN_IN
            || connections
                .iter()
                .find(|x| x.id == c.connection_id)
                .and_then(|x| x.source.as_deref())
                .is_some_and(|s| s != "token" && s != "getkeys"))
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

fn short(plugin_id: Option<&str>) -> String {
    plugin_id.map_or_else(
        || CUSTOM.to_owned(),
        |p| p.strip_prefix("official.").unwrap_or(p).to_owned(),
    )
}

pub fn keys(s: &Snapshot) -> Vec<Key<'_>> {
    let manifest = |id: &str| s.plugins.iter().find(|p| p.manifest.id == id);
    let mut keys: Vec<Key> = s
        .credentials
        .iter()
        .filter(|c| listed(c, &s.connections))
        .filter_map(|c| {
            let key = |provider: &str, plugin_id, account, place: String, secretless| Key {
                c,
                provider: provider.to_owned(),
                plugin_id,
                account,
                place,
                secretless,
                reference: String::new(),
            };
            if c.credential_type == CUSTOM {
                return Some(key("Custom", None, None, "No provider".into(), false));
            }
            if !c.plugin_id.is_empty() {
                let m = &manifest(&c.plugin_id)?.manifest;
                let place = if c.id.starts_with("found:") {
                    "On this device"
                } else {
                    "Added by you"
                };
                return Some(key(&m.name, Some(m.id.clone()), None, place.into(), false));
            }
            let connection = s.connections.iter().find(|x| x.id == c.connection_id)?;
            let m = &manifest(&connection.plugin_id)?.manifest;
            let place = if c.resource.is_empty() {
                connection.name.clone()
            } else if c.resource_name.is_empty() {
                c.resource.clone()
            } else {
                c.resource_name.clone()
            };
            let secretless =
                !c.value_available && !m.operations.contains(&OperationKind::ReadCredential);
            Some(key(
                &m.name,
                Some(m.id.clone()),
                Some(connection.name.clone()),
                place,
                secretless,
            ))
        })
        .collect();
    keys.sort_by(|a, b| {
        (
            a.plugin_id.is_none(),
            &a.provider,
            &a.account,
            &a.place,
            &a.c.name,
        )
            .cmp(&(
                b.plugin_id.is_none(),
                &b.provider,
                &b.account,
                &b.place,
                &b.c.name,
            ))
    });
    let names: Vec<(String, String)> = keys
        .iter()
        .map(|k| (short(k.plugin_id.as_deref()), slug(&k.c.name)))
        .collect();
    for (k, (provider, name)) in keys.iter_mut().zip(&names) {
        let unique = !name.is_empty()
            && names
                .iter()
                .filter(|n| n.0 == *provider && n.1 == *name)
                .count()
                == 1;
        let name = if unique { name } else { &k.c.id };
        k.reference = format!("{SCHEME}{provider}/{name}");
    }
    keys
}

impl Key<'_> {
    /// The `ls --json` entry. Values are never included.
    pub fn json(&self) -> Value {
        json!({
            "id": self.c.id,
            "ref": self.reference,
            "name": self.c.name,
            "provider": self.provider,
            "plugin_id": self.plugin_id,
            "account": self.account,
            "groups": self.c.groups,
            "fields": self.c.fields.iter().map(|f| json!({"key": f.key, "label": f.label, "secret": f.secret})).collect::<Vec<_>>(),
            "problem": self.c.problem,
        })
    }
    pub fn status(&self) -> &str {
        if let Some(problem) = &self.c.problem {
            problem
        } else if self.secretless || !self.c.value_available {
            "no value"
        } else {
            "ok"
        }
    }
    pub fn in_group(&self, group: &str) -> bool {
        self.c.groups.iter().any(|g| g.eq_ignore_ascii_case(group))
    }
    /// The field a reference or `--field` names, else the key's main value.
    pub fn field(&self, name: Option<&str>) -> Result<String> {
        let fields = &self.c.fields;
        let found = match name {
            Some(n) => fields
                .iter()
                .find(|f| f.key == n)
                .or_else(|| fields.iter().find(|f| f.env == n))
                .or_else(|| {
                    fields
                        .iter()
                        .find(|f| f.key.eq_ignore_ascii_case(n) || f.label.eq_ignore_ascii_case(n))
                }),
            None => fields
                .iter()
                .find(|f| f.secret && f.available)
                .or_else(|| fields.iter().find(|f| f.available)),
        };
        let field = found.with_context(|| {
            let known: Vec<_> = fields.iter().map(|f| f.key.as_str()).collect();
            format!(
                "{} has no field {}. Fields: {}",
                self.c.name,
                name.unwrap_or("with a value"),
                known.join(", ")
            )
        })?;
        ensure!(
            field.available,
            "{} isn't stored on this device",
            field.label
        );
        Ok(field.key.clone())
    }
}

/// A key named by a `getkeys://provider/key[/field]` reference or by its ID, with the field the
/// reference names.
pub fn find<'a, 'k>(keys: &'a [Key<'k>], reference: &str) -> Result<(&'a Key<'k>, Option<String>)> {
    let Some(path) = reference.strip_prefix(SCHEME) else {
        let key = keys.iter().find(|k| k.c.id == reference).with_context(|| {
            format!("No key has the ID {reference}. Run getkeys ls to see keys.")
        })?;
        return Ok((key, None));
    };
    let mut parts = path.trim_end_matches('/').splitn(3, '/');
    let (Some(provider), Some(name)) = (parts.next(), parts.next()) else {
        bail!("Write references as getkeys://provider/key or getkeys://provider/key/field");
    };
    let field = parts.next().filter(|f| !f.is_empty()).map(str::to_owned);
    let matches: Vec<&Key> = keys
        .iter()
        .filter(|k| {
            short(k.plugin_id.as_deref()).eq_ignore_ascii_case(provider)
                && (slug(&k.c.name) == slug(name) || k.c.id == name)
        })
        .collect();
    match matches[..] {
        [key] => Ok((key, field)),
        [] => bail!("No key matches {reference}. Run getkeys ls to see references."),
        _ => bail!(
            "{reference} matches {} keys. Use one of: {}",
            matches.len(),
            matches
                .iter()
                .map(|k| format!("{SCHEME}{provider}/{}", k.c.id))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Keys in any of `groups` plus those `references` name, in list order, each with the field
/// its reference picked (if any).
pub fn select<'a, 'k>(
    keys: &'a [Key<'k>],
    groups: &[String],
    references: &[String],
) -> Result<Vec<(&'a Key<'k>, Option<String>)>> {
    let mut chosen: Vec<(&Key, Option<String>)> = Vec::new();
    for group in groups {
        ensure!(
            keys.iter().any(|k| k.in_group(group)),
            "No keys are in the group {group}"
        );
        for k in keys.iter().filter(|k| k.in_group(group)) {
            if !chosen.iter().any(|(c, _)| c.c.id == k.c.id) {
                chosen.push((k, None));
            }
        }
    }
    for reference in references {
        let (key, field) = find(keys, reference)?;
        chosen.retain(|(c, _)| c.c.id != key.c.id);
        chosen.push((key, field));
    }
    Ok(chosen)
}

fn token(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_uppercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_owned()
}

/// Variable names for the chosen keys, as the app's Export names them: names used twice get the
/// key's place (or, without a provider, its name) added.
pub fn env_lines(items: &[(&Key, Option<String>)]) -> Vec<EnvLine> {
    let all: Vec<(&Key, &str, &str)> = items
        .iter()
        .filter(|(k, _)| !k.secretless)
        .flat_map(|(k, only)| {
            k.c.fields
                .iter()
                .filter(move |f| {
                    !f.env.is_empty() && f.available && only.as_ref().is_none_or(|o| *o == f.key)
                })
                .map(move |f| (*k, f.key.as_str(), f.env.as_str()))
        })
        .collect();
    let mut used = std::collections::BTreeSet::new();
    all.iter()
        .map(|(k, field, env)| {
            let mut variable = env.to_string();
            if all.iter().filter(|x| x.2 == *env).count() > 1 {
                let place = if k.place == "No provider" {
                    &k.c.name
                } else {
                    &k.place
                };
                let place = token(place);
                let base = format!("{env}_{}", if place.is_empty() { "KEY" } else { &place });
                variable = base.clone();
                let mut n = 2;
                while used.contains(&variable) {
                    variable = format!("{base}_{n}");
                    n += 1;
                }
            }
            used.insert(variable.clone());
            EnvLine {
                credential_id: k.c.id.clone(),
                field: field.to_string(),
                variable,
            }
        })
        .collect()
}
