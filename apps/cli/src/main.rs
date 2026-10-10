mod client;
mod keys;
mod mcp;
mod native;
mod new;
mod ops;

use anyhow::{Context, Result, bail, ensure};
use clap::{CommandFactory, Parser, Subcommand};
use client::Client;
use getkeys_core::{
    App,
    commands::Command,
    discover::Discovery,
    model::{CredentialView, CustomField, Deletion, Snapshot},
};
use keys::Key;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{IsTerminal, Read, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    name = "getkeys",
    version,
    about = "Your API keys from GetKeys, for scripts, terminals and agents"
)]
struct Args {
    /// Folder holding the vault.
    #[arg(long, env = "GETKEYS_DATA_DIR", global = true)]
    data_dir: Option<PathBuf>,
    /// Provider catalog to use when GetKeys isn't running.
    #[arg(long, env = "GETKEYS_REGISTRY", global = true, hide = true)]
    registry: Option<PathBuf>,
    #[command(subcommand)]
    command: Cli,
}

#[derive(Subcommand)]
enum Cli {
    /// List keys with their references. Values are never shown.
    Ls {
        #[arg(long)]
        group: Option<String>,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Print one value. Prefer `run` or `env` so values stay out of your terminal.
    Get {
        /// getkeys://provider/key[/field] or a key ID
        reference: String,
        #[arg(long)]
        field: Option<String>,
        /// Print the value
        #[arg(long)]
        show: bool,
    },
    /// Run a program with keys in its environment (no shell). getkeys:// references in
    /// variables and in --env-file are replaced with their values.
    Run {
        #[arg(long)]
        group: Vec<String>,
        #[arg(long = "key", value_name = "REF")]
        keys: Vec<String>,
        #[arg(long)]
        env_file: Option<PathBuf>,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Print keys as a .env file, or write it with -o.
    Env {
        #[arg(long)]
        group: Vec<String>,
        #[arg(long = "key", value_name = "REF")]
        keys: Vec<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Replace getkeys:// references (bare or in {{ }}) in a file with their values.
    Inject {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Look for new keys and list each account's keys.
    Refresh {
        #[arg(long)]
        group: Option<String>,
    },
    /// Have the provider replace a key with a new one.
    Rotate { reference: String },
    /// Ask the provider whether a key still works.
    Check { reference: String },
    /// Save a key. Its value is read from stdin or asked for.
    Add {
        /// Provider ID, such as openai
        #[arg(long, required_unless_present = "custom")]
        provider: Option<String>,
        #[arg(long)]
        name: Option<String>,
        /// A provider field, as key=value
        #[arg(long = "field", value_parser = parse_pair)]
        fields: Vec<(String, String)>,
        /// A key without a provider: VARIABLE=value, or VARIABLE to read the value
        #[arg(long, conflicts_with = "provider")]
        custom: Vec<String>,
        #[arg(long)]
        group: Vec<String>,
    },
    /// Delete a key.
    Rm {
        reference: String,
        /// Finish deleting a key you revoked on the provider's page
        #[arg(long)]
        confirm: bool,
    },
    /// Ask a provider for a new key and save it.
    New {
        /// Provider ID, such as openai
        provider: String,
        #[arg(long)]
        name: Option<String>,
        /// The account to create it in, when several are connected
        #[arg(long)]
        account: Option<String>,
        /// Where the key belongs, for providers whose keys belong to a project, workspace or
        /// IAM user
        #[arg(long, conflicts_with = "new_project")]
        project: Option<String>,
        /// Make a new project, workspace or IAM user with this name for the key
        #[arg(long, value_name = "NAME")]
        new_project: Option<String>,
        /// The organisation that owns the new project
        #[arg(long, requires = "new_project")]
        owner: Option<String>,
        /// Where the new project runs, such as americas, emea or apac
        #[arg(long, requires = "new_project")]
        region: Option<String>,
        /// A choice for the new project, such as policy=ReadOnlyAccess or group=developers
        #[arg(long = "option", value_name = "KEY=VALUE", requires = "new_project")]
        options: Vec<String>,
        #[arg(long)]
        group: Vec<String>,
    },
    /// Manage providers.
    Providers {
        #[command(subcommand)]
        action: Providers,
    },
    /// Save an encrypted backup of the vault.
    Backup { path: PathBuf },
    /// Restore a backup into an empty vault.
    Restore { path: PathBuf },
    /// Print a shell completion script.
    Completions { shell: clap_complete::Shell },
    /// Serve the Model Context Protocol on stdin/stdout for AI agents.
    Mcp,
    /// Set up the GetKeys browser extension.
    NativeHost {
        #[command(subcommand)]
        action: Host,
    },
}

#[derive(Subcommand)]
enum Providers {
    Ls,
    /// Add a provider and save its keys found on this device.
    Add {
        id: String,
    },
    /// Remove a provider with its accounts and keys.
    Rm {
        id: String,
    },
    /// Open the provider's page for creating keys.
    Open {
        id: String,
    },
}

#[derive(Subcommand)]
enum Host {
    Install {
        #[arg(long, value_enum, default_value = "all")]
        browser: native::Browser,
    },
    Uninstall {
        #[arg(long, value_enum, default_value = "all")]
        browser: native::Browser,
    },
}

fn parse_pair(input: &str) -> Result<(String, String), String> {
    input
        .split_once('=')
        .map(|(k, v)| (k.trim().into(), v.into()))
        .ok_or_else(|| "Expected key=value".into())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn data_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    explicit.map_or_else(App::data_dir, Ok)
}

fn run() -> Result<()> {
    // Browsers start a native messaging host with the caller's origin as its first argument.
    if let Some(origin) = std::env::args()
        .nth(1)
        .filter(|a| a.starts_with("chrome-extension://"))
    {
        let dir = data_dir(std::env::var_os("GETKEYS_DATA_DIR").map(PathBuf::from))?;
        let registry = std::env::var_os("GETKEYS_REGISTRY").map(PathBuf::from);
        return native::serve(&mut Client::new(dir, registry), &origin);
    }
    let args = Args::parse();
    let dir = data_dir(args.data_dir)?;
    let client = &mut Client::new(dir.clone(), args.registry);
    match args.command {
        Cli::Ls {
            group,
            provider,
            json,
        } => {
            let snapshot = client.snapshot()?;
            let keys = keys::keys(&snapshot);
            let keys = filter(&keys, group.as_deref(), provider.as_deref());
            if json {
                let entries: Vec<Value> = keys.iter().map(|k| k.json()).collect();
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if keys.is_empty() {
                eprintln!("No keys. Add one with getkeys add, or in GetKeys.");
            } else {
                let rows: Vec<[String; 5]> = keys
                    .iter()
                    .map(|k| {
                        [
                            k.reference.clone(),
                            k.c.name.clone(),
                            k.provider.clone(),
                            k.c.groups.join(", "),
                            k.status().into(),
                        ]
                    })
                    .collect();
                table(["REF", "NAME", "PROVIDER", "GROUPS", "STATUS"], &rows);
            }
        }
        Cli::Get {
            reference,
            field,
            show,
        } => {
            ensure!(
                show,
                "Add --show to print the value. To give keys to a program, use getkeys run or getkeys env."
            );
            println!(
                "{}",
                ops::value(client, &reference, field.as_deref(), None)?.0
            );
        }
        Cli::Run {
            group,
            keys,
            env_file,
            command,
        } => {
            let env = ops::environment(client, &group, &keys, env_file.as_deref(), None)?;
            let status = std::process::Command::new(ops::program(&command[0]))
                .args(&command[1..])
                .envs(&env.vars)
                .status()
                .with_context(|| format!("Unable to start {}", command[0]))?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Cli::Env {
            group,
            keys,
            output,
        } => {
            let text = ops::dotenv(client, &group, &keys, None)?;
            write_out(output, &text)?;
        }
        Cli::Inject { input, output } => {
            let text = std::fs::read_to_string(&input)
                .with_context(|| format!("Unable to read {}", input.display()))?;
            let snapshot = client.snapshot()?;
            let mut env = ops::Env::default();
            let text = ops::substitute(client, &snapshot, &[text], None, &mut env)?.remove(0);
            write_out(output, &text)?;
        }
        Cli::Refresh { group } => {
            let summary = ops::refresh(client, group.as_deref(), &mut |p| eprintln!("{p}"))?;
            println!("{summary}");
        }
        Cli::Rotate { reference } => {
            let id = key_id(client, &reference)?;
            let c: CredentialView = client.get(Command::RotateNow { credential_id: id })?;
            println!("Rotated {}", c.name);
        }
        Cli::Check { reference } => {
            let id = key_id(client, &reference)?;
            let problem: Option<String> = client.get(Command::Check { credential_id: id })?;
            match problem {
                Some(problem) => bail!("{problem}"),
                None => println!("The key works"),
            }
        }
        Cli::Add {
            provider,
            name,
            fields,
            custom,
            group,
        } => {
            let saved = match provider {
                Some(provider) => add_key(client, &provider, name, fields)?,
                None => add_custom(client, name, custom)?,
            };
            saved_as(client, saved, group)?;
        }
        Cli::Rm { reference, confirm } => {
            let id = key_id(client, &reference)?;
            if confirm {
                client.call(Command::ConfirmDeleted { credential_id: id })?;
                println!("Deleted");
            } else {
                match client.get(Command::Delete { credential_id: id })? {
                    Deletion::Deleted => println!("Deleted"),
                    Deletion::Manual { url } => println!(
                        "Revoke the key at {url}, then run getkeys rm --confirm {reference}"
                    ),
                }
            }
        }
        Cli::New {
            provider,
            name,
            account,
            project,
            new_project,
            owner,
            region,
            options,
            group,
        } => {
            let snapshot = client.snapshot()?;
            let request = new::Request {
                plugin_id: plugin_id(&snapshot, &provider)?,
                name,
                account,
                project,
                new_project,
                owner,
                region,
                options,
            };
            let saved = new::run(client, request, &mut |line| eprintln!("{line}"))?;
            saved_as(client, saved, group)?;
        }
        Cli::Providers { action } => providers(client, action)?,
        Cli::Backup { path } => {
            let passphrase = passphrase("Backup passphrase (12+ characters): ", true)?;
            client.call(Command::Export {
                path: absolute(path)?,
                passphrase,
            })?;
            println!("Backup saved");
        }
        Cli::Restore { path } => {
            let passphrase = passphrase("Backup passphrase: ", false)?;
            client.call(Command::Restore {
                path: absolute(path)?,
                passphrase,
            })?;
            println!("Backup restored");
        }
        Cli::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Args::command(),
                "getkeys",
                &mut std::io::stdout(),
            );
        }
        Cli::Mcp => mcp::serve(client)?,
        Cli::NativeHost { action } => match action {
            Host::Install { browser } => {
                for place in native::install(browser, &dir)? {
                    println!("Registered {place}");
                }
            }
            Host::Uninstall { browser } => {
                native::uninstall(browser, &dir)?;
                println!("Removed the browser extension host");
            }
        },
    }
    Ok(())
}

fn filter<'a, 'k>(
    keys: &'a [Key<'k>],
    group: Option<&str>,
    provider: Option<&str>,
) -> Vec<&'a Key<'k>> {
    keys.iter()
        .filter(|k| group.is_none_or(|g| k.in_group(g)))
        .filter(|k| {
            provider.is_none_or(|p| {
                let id = k.plugin_id.as_deref().unwrap_or(getkeys_core::app::CUSTOM);
                id.eq_ignore_ascii_case(p)
                    || id
                        .strip_prefix("official.")
                        .is_some_and(|s| s.eq_ignore_ascii_case(p))
                    || k.provider.eq_ignore_ascii_case(p)
            })
        })
        .collect()
}

/// The `ls --json` entries.
pub fn list(
    client: &mut Client,
    group: Option<String>,
    provider: Option<String>,
) -> Result<Vec<Value>> {
    let snapshot = client.snapshot()?;
    let keys = keys::keys(&snapshot);
    Ok(filter(&keys, group.as_deref(), provider.as_deref())
        .iter()
        .map(|k| k.json())
        .collect())
}

fn table<const N: usize>(headers: [&str; N], rows: &[[String; N]]) {
    let mut widths = headers.map(str::len);
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(widths)
            .map(|(c, w)| format!("{c:w$}"))
            .collect();
        println!("{}", padded.join("  ").trim_end());
    };
    line(headers.to_vec());
    for row in rows {
        line(row.iter().map(String::as_str).collect());
    }
}

fn write_out(output: Option<PathBuf>, text: &str) -> Result<()> {
    match output {
        Some(path) => std::fs::write(&path, text)
            .with_context(|| format!("Unable to write {}", path.display())),
        None => Ok(std::io::stdout().write_all(text.as_bytes())?),
    }
}

fn absolute(path: PathBuf) -> Result<String> {
    Ok(std::path::absolute(path)?.display().to_string())
}

/// Files a new key under its groups and prints its reference.
fn saved_as(client: &mut Client, saved: CredentialView, group: Vec<String>) -> Result<()> {
    if !group.is_empty() {
        client.call(Command::SetGroups {
            credential_id: saved.id.clone(),
            groups: group,
        })?;
    }
    let snapshot = client.snapshot()?;
    let reference = keys::keys(&snapshot)
        .iter()
        .find(|k| k.c.id == saved.id)
        .map_or_else(|| saved.id.clone(), |k| k.reference.clone());
    println!("Saved {} as {reference}", saved.name);
    Ok(())
}

fn key_id(client: &mut Client, reference: &str) -> Result<String> {
    let snapshot = client.snapshot()?;
    let keys = keys::keys(&snapshot);
    Ok(keys::find(&keys, reference)?.0.c.id.clone())
}

/// Everything piped to stdin, if anything is.
fn piped() -> Result<Option<String>> {
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut text = String::new();
    std::io::stdin()
        .take(1024 * 1024)
        .read_to_string(&mut text)?;
    Ok(Some(text.trim().to_owned()).filter(|t| !t.is_empty()))
}

fn ask(label: &str, secret: bool) -> Result<String> {
    ensure!(
        std::io::stdin().is_terminal(),
        "Pipe the value for {label} to stdin"
    );
    if secret {
        return Ok(rpassword::prompt_password(format!("{label}: "))?);
    }
    eprint!("{label}: ");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

fn passphrase(prompt: &str, confirm: bool) -> Result<String> {
    if let Some(text) = piped()? {
        return Ok(text.lines().next().unwrap_or_default().to_owned());
    }
    let first = rpassword::prompt_password(prompt)?;
    if confirm {
        ensure!(
            rpassword::prompt_password("Repeat the passphrase: ")? == first,
            "The passphrases don't match"
        );
    }
    Ok(first)
}

fn plugin_id(snapshot: &Snapshot, id: &str) -> Result<String> {
    snapshot
        .plugins
        .iter()
        .map(|p| &p.manifest)
        .find(|m| {
            m.id == id
                || m.id.strip_prefix("official.") == Some(id)
                || m.name.eq_ignore_ascii_case(id)
        })
        .map(|m| m.id.clone())
        .with_context(|| format!("No provider is called {id}. See getkeys providers ls."))
}

fn add_key(
    client: &mut Client,
    provider: &str,
    name: Option<String>,
    given: Vec<(String, String)>,
) -> Result<CredentialView> {
    let snapshot = client.snapshot()?;
    let plugin_id = plugin_id(&snapshot, provider)?;
    let manifest = &snapshot
        .plugins
        .iter()
        .find(|p| p.manifest.id == plugin_id)
        .context("Unknown provider")?
        .manifest;
    let mut fields: BTreeMap<String, String> = given.into_iter().collect();
    let mut piped = piped()?;
    for f in manifest
        .credential_fields
        .iter()
        .filter(|f| f.required || f.secret)
    {
        if fields.contains_key(&f.key) {
            continue;
        }
        let value = match piped.take() {
            Some(v) => v,
            None if f.required => ask(&f.label, f.secret)?,
            None => continue,
        };
        fields.insert(f.key.clone(), value);
    }
    client.get(Command::SaveKey {
        credential_id: None,
        plugin_id,
        name: name.unwrap_or_default(),
        fields,
    })
}

fn add_custom(
    client: &mut Client,
    name: Option<String>,
    custom: Vec<String>,
) -> Result<CredentialView> {
    let name = name.context("Give the key a --name")?;
    let mut piped = piped()?;
    let mut fields = Vec::new();
    for item in custom {
        let (variable, value) = match item.split_once('=') {
            Some((k, v)) => (k.trim().to_owned(), v.to_owned()),
            None => {
                let value = match piped.take() {
                    Some(v) => v,
                    None => ask(&item, true)?,
                };
                (item.trim().to_owned(), value)
            }
        };
        fields.push(CustomField {
            variable,
            value,
            secret: true,
        });
    }
    client.get(Command::SaveCustom {
        credential_id: None,
        name,
        fields,
    })
}

fn providers(client: &mut Client, action: Providers) -> Result<()> {
    let snapshot = client.snapshot()?;
    match action {
        Providers::Ls => {
            let rows: Vec<[String; 4]> = snapshot
                .plugins
                .iter()
                .map(|p| {
                    let m = &p.manifest;
                    let status = match (p.installed, p.update_available) {
                        (true, true) => "added, update available",
                        (true, false) => "added",
                        _ => "",
                    };
                    [
                        m.id.strip_prefix("official.").unwrap_or(&m.id).into(),
                        m.name.clone(),
                        m.category.clone(),
                        status.into(),
                    ]
                })
                .collect();
            table(["ID", "NAME", "CATEGORY", "STATUS"], &rows);
        }
        Providers::Add { id } => {
            let plugin_id = plugin_id(&snapshot, &id)?;
            client.call(Command::Install {
                plugin_id: plugin_id.clone(),
                accept_permissions: true,
            })?;
            let found: Vec<Discovery> = client.get(Command::ScanSources {
                plugin_id: Some(plugin_id.clone()),
            })?;
            let mut saved = 0;
            for d in found {
                match client.call(Command::ConnectLocal {
                    source_id: d.id,
                    connection_id: None,
                }) {
                    Ok(_) => saved += 1,
                    Err(e) => eprintln!("{}: {e:#}", d.source),
                }
            }
            println!("Added {plugin_id}; saved {saved} found on this device");
        }
        Providers::Rm { id } => {
            client.call(Command::Uninstall {
                plugin_id: plugin_id(&snapshot, &id)?,
            })?;
            println!("Removed {id}");
        }
        Providers::Open { id } => {
            client.call(Command::OpenTokenPage {
                plugin_id: plugin_id(&snapshot, &id)?,
            })?;
        }
    }
    Ok(())
}
