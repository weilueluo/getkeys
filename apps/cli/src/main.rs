use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use getkeys_core::{App, commands::Command, model::AuthProgress};
use std::{
    collections::BTreeMap,
    io::{IsTerminal, Read},
    path::PathBuf,
};

#[derive(Parser)]
#[command(version, about = "Your provider credentials, kept locally")]
struct Args {
    #[arg(long, env = "GETKEYS_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[arg(long, env = "GETKEYS_REGISTRY", default_value = "registry/dist")]
    registry: PathBuf,
    #[command(subcommand)]
    command: Cli,
}
#[derive(Subcommand)]
enum Cli {
    /// List plugins, accounts, and credential metadata. Values are never included.
    List,
    Install {
        plugin: String,
        #[arg(long)]
        accept_permissions: bool,
    },
    /// Open the browser, then finish authorization in this terminal.
    Connect {
        plugin: String,
        #[arg(long,value_parser=parse_pair)]
        config: Vec<(String, String)>,
    },
    /// Run a typed application command from JSON on stdin.
    Rpc,
    /// Print a secret field only with explicit stdout consent.
    Get {
        id: String,
        #[arg(long, default_value = "token")]
        field: String,
        #[arg(long)]
        show: bool,
    },
    /// Inject local bindings into a child process, without a shell or .env file.
    Run {
        #[arg(long)]
        project: String,
        #[arg(long, default_value = "development")]
        environment: String,
        #[arg(required = true, trailing_var_arg = true)]
        command: Vec<String>,
    },
    Export {
        path: PathBuf,
    },
    Restore {
        path: PathBuf,
    },
    /// Download a signed marketplace to a new directory.
    Marketplace {
        url: String,
        destination: PathBuf,
    },
}
fn parse_pair(input: &str) -> Result<(String, String), String> {
    input
        .split_once('=')
        .map(|(k, v)| (k.into(), v.into()))
        .ok_or_else(|| "Expected key=value".into())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args = Args::parse();
    if let Cli::Marketplace { url, destination } = &args.command {
        return getkeys_core::plugins::Registry::download(url, destination);
    }
    let app = App::open(args.data_dir.unwrap_or(App::data_dir()?), args.registry)?;
    match args.command {
        Cli::List => println!("{}", serde_json::to_string_pretty(&app.snapshot()?)?),
        Cli::Install {
            plugin,
            accept_permissions,
        } => app.install(&plugin, accept_permissions)?,
        Cli::Connect { plugin, config } => {
            let mut progress = app.begin_auth(&plugin, BTreeMap::from_iter(config))?;
            let mut flow_id = String::new();
            loop {
                match progress {
                    AuthProgress::Browser {
                        flow_id: id,
                        url,
                        user_code,
                        interval,
                    } => {
                        flow_id = id;
                        eprintln!("Authorize in your browser: {url}");
                        if let Some(code) = user_code {
                            eprintln!("Device code: {code}");
                        }
                        std::thread::sleep(std::time::Duration::from_secs(interval));
                        progress = app.poll_auth(&flow_id)?;
                    }
                    AuthProgress::Pending { interval } => {
                        std::thread::sleep(std::time::Duration::from_secs(interval));
                        progress = app.poll_auth(&flow_id)?;
                    }
                    AuthProgress::SelectRole { flow_id: id, roles } => {
                        for (i, r) in roles.iter().enumerate() {
                            eprintln!("{}: {} / {}", i + 1, r.account_name, r.role);
                        }
                        eprintln!("Choose a role number:");
                        let mut choice = String::new();
                        std::io::stdin().read_line(&mut choice)?;
                        let selected = choice
                            .trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|n| n.checked_sub(1))
                            .and_then(|i| roles.get(i))
                            .context("Invalid role selection")?;
                        progress = app.choose_role(&id, &selected.account_id, &selected.role)?;
                    }
                    AuthProgress::Connected { connection } => {
                        println!("Connected {} ({})", connection.name, connection.id);
                        break;
                    }
                }
            }
        }
        Cli::Rpc => {
            ensure!(
                !std::io::stdin().is_terminal(),
                "Pipe a command JSON object to stdin"
            );
            let mut input = String::new();
            std::io::stdin()
                .take(1024 * 1024 + 1)
                .read_to_string(&mut input)?;
            ensure!(input.len() <= 1024 * 1024, "Command exceeds size limit");
            let command: Command = serde_json::from_str(&input)?;
            ensure!(
                !matches!(command, Command::Reveal { .. }),
                "Use get --show for deliberate secret output"
            );
            println!("{}", app.dispatch(command)?);
        }
        Cli::Get { id, field, show } => {
            ensure!(
                show,
                "Use --show to deliberately print a secret. Prefer getkeys run for applications."
            );
            let secret = app.secret(&id)?;
            println!(
                "{}",
                secret
                    .fields
                    .get(&field)
                    .context("Credential field not found")?
            );
        }
        Cli::Run {
            project,
            environment,
            command,
        } => {
            let values = app.environment(&project, &environment)?;
            let status = std::process::Command::new(&command[0])
                .args(&command[1..])
                .envs(values)
                .status()
                .context("Unable to start child process")?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Cli::Export { path } => {
            let pass = rpassword::prompt_password("Export passphrase (12+ characters): ")?;
            app.vault.export(&path, pass)?;
        }
        Cli::Restore { path } => {
            let pass = rpassword::prompt_password("Backup passphrase: ")?;
            app.vault.import_backup(&path, pass)?;
        }
        Cli::Marketplace { .. } => bail!("Marketplace command was not handled"),
    }
    Ok(())
}
