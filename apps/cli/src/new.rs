//! `getkeys new`: asks a provider for a new key, in a project made for it when asked. Mirrors New
//! key in the app.
use crate::client::Client;
use anyhow::{Context, Result, bail, ensure};
use getkeys_core::{
    api::{Choice, Manifest, OperationKind, Owner, Resource},
    commands::Command,
    create::Created,
    model::{ConnectionState, ConnectionView, CredentialView, Snapshot},
};
use std::{collections::BTreeMap, thread::sleep, time::Duration};

const WAIT: Duration = Duration::from_secs(5);
const TRIES: usize = 48;

pub struct Request {
    pub plugin_id: String,
    pub name: Option<String>,
    /// An account's name or ID, when several are connected.
    pub account: Option<String>,
    /// The project the key belongs to, by name or ID.
    pub project: Option<String>,
    /// Make a new project with this name for the key.
    pub new_project: Option<String>,
    pub owner: Option<String>,
    pub region: Option<String>,
    /// `KEY=VALUE` choices for the new project.
    pub options: Vec<String>,
}

/// A name every provider accepts: lowercase letters, numbers, and single hyphens.
pub fn slug(value: &str) -> String {
    let mut out = String::new();
    for c in value.to_lowercase().chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            c
        } else {
            '-'
        };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    out.trim_start_matches(['-', '.', '_'])
        .chars()
        .take(100)
        .collect()
}

fn matches(wanted: &str, id: &str, name: &str) -> bool {
    let wanted = wanted.trim().to_lowercase();
    id.to_lowercase() == wanted || name.to_lowercase() == wanted
}

fn account<'a>(
    snapshot: &'a Snapshot,
    plugin_id: &str,
    provider: &str,
    wanted: Option<&str>,
) -> Result<&'a ConnectionView> {
    let accounts: Vec<_> = snapshot
        .connections
        .iter()
        .filter(|c| c.plugin_id == plugin_id && c.state == ConnectionState::Connected)
        .collect();
    let short = plugin_id.strip_prefix("official.").unwrap_or(plugin_id);
    ensure!(
        !accounts.is_empty(),
        "Connect a {provider} account first: getkeys providers add {short}"
    );
    match wanted {
        Some(w) => accounts
            .iter()
            .find(|c| matches(w, &c.id, &c.name))
            .copied()
            .with_context(|| {
                format!(
                    "No {provider} account is called {w}. Choose one of: {}",
                    names(accounts.iter().map(|c| c.name.as_str()))
                )
            }),
        None if accounts.len() == 1 => Ok(accounts[0]),
        None => bail!(
            "Choose a {provider} account with --account: {}",
            names(accounts.iter().map(|c| c.name.as_str()))
        ),
    }
}

fn names<'a>(all: impl Iterator<Item = &'a str>) -> String {
    all.collect::<Vec<_>>().join(", ")
}

/// Asks the provider for a key and saves it. Progress goes to `say`.
pub fn run(
    client: &mut Client,
    request: Request,
    say: &mut dyn FnMut(String),
) -> Result<CredentialView> {
    let snapshot = client.snapshot()?;
    let plugin = snapshot
        .plugins
        .iter()
        .find(|p| p.manifest.id == request.plugin_id)
        .context("Unknown provider")?;
    let manifest = &plugin.manifest;
    let provider = manifest.name.as_str();
    let short = manifest
        .id
        .strip_prefix("official.")
        .unwrap_or(&manifest.id);
    ensure!(
        plugin.installed,
        "Add {provider} first: getkeys providers add {short}"
    );
    ensure!(
        manifest
            .operations
            .contains(&OperationKind::CreateCredential),
        "{provider} doesn't make keys through its API. Create one on its page (getkeys providers open {short}), then save it with getkeys add --provider {short}"
    );
    let account = account(
        &snapshot,
        &manifest.id,
        provider,
        request.account.as_deref(),
    )?
    .clone();
    let noun = manifest
        .new_resource
        .as_ref()
        .map_or("project", |r| r.noun.as_str());
    let resource = match (&request.new_project, &request.project) {
        (Some(_), _) if manifest.account_credentials || manifest.account_keys => {
            bail!("{provider} keys belong to the account, so there is nothing to make for them")
        }
        (Some(name), _) => {
            let page = manifest.new_resource.as_ref().map(|r| r.page.as_str());
            if !manifest.operations.contains(&OperationKind::CreateResource) {
                match page.filter(|p| !p.is_empty()) {
                    Some(page) => bail!(
                        "{provider} can't make a {noun} through its API. Make one at {page}, then run this again with --project <name>"
                    ),
                    None => bail!("{provider} keys don't belong to a {noun} GetKeys can make"),
                }
            }
            let project = new_project(client, &account, manifest, &request, name, say)?;
            project.id
        }
        (None, Some(wanted)) => {
            let all: Vec<Resource> = client.get(Command::Resources {
                connection_id: account.id.clone(),
            })?;
            all.iter()
                .find(|r| matches(wanted, &r.id, &r.name))
                .map(|r| r.id.clone())
                .with_context(|| {
                    format!(
                        "No {noun} is called {wanted}. Choose one of: {}",
                        names(all.iter().map(|r| r.name.as_str()))
                    )
                })?
        }
        (None, None) if manifest.account_credentials || manifest.account_keys => String::new(),
        (None, None) => {
            let all: Vec<Resource> = client.get(Command::Resources {
                connection_id: account.id.clone(),
            })?;
            match all.as_slice() {
                [] => bail!("This account has no {noun} yet. Make one with --new-project <name>"),
                [only] => only.id.clone(),
                _ => bail!(
                    "Choose where the key belongs with --project: {}",
                    names(all.iter().map(|r| r.name.as_str()))
                ),
            }
        }
    };
    let name = request
        .name
        .clone()
        .or_else(|| request.new_project.clone())
        .unwrap_or_else(|| "GetKeys".into());
    say(format!("Creating {name} in {provider}"));
    client.get(Command::Create {
        connection_id: account.id,
        resource,
        name,
    })
}

/// Makes a project for the key and waits until it can make keys.
fn new_project(
    client: &mut Client,
    account: &ConnectionView,
    manifest: &Manifest,
    request: &Request,
    name: &str,
    say: &mut dyn FnMut(String),
) -> Result<Resource> {
    let template = manifest
        .new_resource
        .as_ref()
        .with_context(|| format!("{} can't make anything for its keys", manifest.name))?;
    let noun = template.noun.as_str();
    let project = slug(name);
    ensure!(!project.is_empty(), "Choose a name with letters or numbers");
    let owner = if manifest.operations.contains(&OperationKind::ListOwners) {
        let owners: Vec<Owner> = client.get(Command::Owners {
            connection_id: account.id.clone(),
        })?;
        match &request.owner {
            Some(w) => owners
                .iter()
                .find(|o| matches(w, &o.id, &o.name))
                .with_context(|| {
                    format!(
                        "No organisation is called {w}. Choose one of: {}",
                        names(owners.iter().map(|o| o.name.as_str()))
                    )
                })?,
            None => owners
                .first()
                .with_context(|| format!("This account can't make a {noun}"))?,
        }
        .id
        .clone()
    } else {
        ensure!(
            request.owner.is_none(),
            "{} has no organisations to choose from",
            manifest.name
        );
        String::new()
    };
    let mut options = BTreeMap::new();
    if let Some(region) = &request.region {
        options.insert("region".to_owned(), region.clone());
    }
    for pair in &request.options {
        let (key, value) = pair
            .split_once('=')
            .with_context(|| format!("Write --option as KEY=VALUE, not {pair}"))?;
        let option = template
            .options
            .iter()
            .find(|o| o.key == key.trim())
            .with_context(|| {
                format!(
                    "A {noun} has no option {key}. Choose one of: {}",
                    names(template.options.iter().map(|o| o.key.as_str()))
                )
            })?;
        let choices = if option.listed {
            client.get::<Vec<Choice>>(Command::Choices {
                connection_id: account.id.clone(),
                option: option.key.clone(),
            })?
        } else {
            option.choices.clone()
        };
        let value = value.trim();
        let chosen = choices
            .iter()
            .find(|c| matches(value, &c.value, &c.label) || matches(value, &c.value, label(c)))
            .with_context(|| {
                let some: Vec<_> = choices.iter().take(20).map(label).collect();
                format!(
                    "No {} is called {value}. Choose one of: {}",
                    option.label.to_lowercase(),
                    some.join(", ")
                )
            })?;
        options.insert(option.key.clone(), chosen.value.clone());
    }
    say(format!("Making the {noun} {project}"));
    let created: Created = client.get(Command::CreateResource {
        connection_id: account.id.clone(),
        owner,
        name: project,
        options,
    })?;
    if let Some(saved) = &created.generated {
        say(format!("Saved its generated secret as {}", saved.name));
    }
    if !created.resource.note.is_empty() {
        say(created.resource.note.clone());
    }
    if created.resource.pending {
        say("Starting it up. This takes a minute or two.".into());
        wait_for(client, &account.id, &created.resource.id)?;
    }
    Ok(created.resource)
}

/// A choice's label without notes such as "(AWS managed)".
fn label(c: &Choice) -> &str {
    c.label.split(" (").next().unwrap_or(&c.label)
}

fn wait_for(client: &mut Client, account: &str, reference: &str) -> Result<()> {
    for _ in 0..TRIES {
        sleep(WAIT);
        let all: Vec<Resource> = client.get(Command::Resources {
            connection_id: account.into(),
        })?;
        if all.iter().any(|r| r.id == reference && !r.pending) {
            return Ok(());
        }
    }
    bail!(
        "The project is still starting. Run getkeys new again with --project {reference} in a few minutes."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_slugs() {
        assert_eq!(slug("My App!"), "my-app-");
        assert_eq!(slug("--web.v2"), "web.v2");
        assert_eq!(slug("a  b"), "a-b");
    }

    #[test]
    fn names_and_ids_match_without_case() {
        assert!(matches("My App", "abc", "my app"));
        assert!(matches(" ABC ", "abc", "other"));
        assert!(!matches("app", "abc", "my app"));
    }
}
