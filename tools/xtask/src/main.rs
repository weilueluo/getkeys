use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use getkeys_core::{
    agents::{AgentClient, AgentInfo},
    api::*,
    commands::Command,
    create::Created,
    discover::Discovery,
    model::*,
    plugins::{Catalog, Package, verify_catalog},
    prefs::Prefs,
};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use ts_rs::TS;
mod live;

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("live") => live::run()?,
        Some("init-signing") => {
            let path = Path::new(".local/registry-signing-key.hex");
            ensure!(!path.exists(), "A local signing key already exists");
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes)
                .map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
            let key = SigningKey::from_bytes(&bytes);
            fs::create_dir_all(".local")?;
            fs::write(path, hex::encode(bytes))?;
            fs::write(
                "registry/public-key.hex",
                hex::encode(key.verifying_key().as_bytes()),
            )?;
            println!(
                "Created a local signing key under .local and updated registry/public-key.hex. Keep the private key out of Git."
            );
        }
        Some("package") => package()?,
        Some("verify") => {
            let catalog = verify_catalog(
                &fs::read("registry/dist/index.json")?,
                &fs::read_to_string("registry/dist/index.sig")?,
                &fs::read_to_string("registry/public-key.hex")?,
            )?;
            for p in catalog.packages {
                let bytes = if p.declarative() {
                    getkeys_core::plugins::manifest_bytes(&p.manifest)
                } else {
                    fs::read(Path::new("registry/dist").join(&p.wasm))?
                };
                getkeys_core::plugins::verify_artifact(&p, &bytes)?;
            }
            println!("Marketplace signature and all plugin artifacts verified.");
        }
        Some("types") => types()?,
        Some("new-provider") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            let declarative = args.iter().any(|a| a == "--declarative");
            let [id] = args
                .iter()
                .filter(|a| !a.starts_with("--"))
                .collect::<Vec<_>>()[..]
            else {
                bail!("Usage: cargo run -p xtask -- new-provider <id> [--declarative]");
            };
            new_provider(id, declarative)?;
        }
        _ => bail!(
            "Usage: cargo run -p xtask -- <init-signing|package|verify|types|live|new-provider <id> [--declarative]>"
        ),
    }
    Ok(())
}
fn package() -> Result<()> {
    let private = std::env::var("GETKEYS_REGISTRY_SIGNING_KEY")
        .or_else(|_| fs::read_to_string(".local/registry-signing-key.hex"))
        .context(
            "Set GETKEYS_REGISTRY_SIGNING_KEY, or run xtask init-signing for a development catalog",
        )?;
    let raw: [u8; 32] = hex::decode(private.trim())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Signing key must be 32 bytes encoded as hex"))?;
    let key = SigningKey::from_bytes(&raw);
    ensure!(
        hex::encode(key.verifying_key().as_bytes())
            == fs::read_to_string("registry/public-key.hex")?.trim(),
        "Signing key does not match the checked-in trust root"
    );
    fs::create_dir_all("registry/dist")?;
    let mut packages = Vec::new();
    let mut providers = fs::read_dir("plugins")?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    providers.retain(|p| p.is_dir());
    providers.sort();
    for provider in providers {
        let id = provider
            .file_name()
            .and_then(|s| s.to_str())
            .context("Invalid provider directory")?;
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(format!("plugins/{id}/manifest.json"))?)
                .with_context(|| format!("plugins/{id}/manifest.json"))?;
        getkeys_core::plugins::validate_manifest(&manifest)
            .with_context(|| format!("plugins/{id}/manifest.json"))?;
        let declarative = manifest.kind == ProviderKind::Declarative;
        ensure!(
            declarative != provider.join("Cargo.toml").exists(),
            "plugins/{id}: declarative providers have only a manifest; managed ones have code"
        );
        let (wasm, sha256) = if declarative {
            let bytes = getkeys_core::plugins::manifest_bytes(&manifest);
            (String::new(), hex::encode(Sha256::digest(&bytes)))
        } else {
            let bytes = fs::read(format!(
                "target/wasm32-unknown-unknown/release/getkeys_plugin_{}.wasm",
                id.replace('-', "_")
            ))?;
            let wasm = format!("{}-{}.wasm", manifest.id, manifest.version);
            fs::write(Path::new("registry/dist").join(&wasm), &bytes)?;
            (wasm, hex::encode(Sha256::digest(&bytes)))
        };
        packages.push(Package {
            manifest,
            wasm,
            sha256,
        });
    }
    // Drop artifacts of providers that were removed or bumped.
    for entry in fs::read_dir("registry/dist")? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".wasm") && !packages.iter().any(|p| p.wasm == name) {
            fs::remove_file(path)?;
        }
    }
    let bytes = serde_json::to_vec_pretty(&Catalog { packages })?;
    fs::write(
        "registry/dist/index.sig",
        STANDARD.encode(key.sign(&bytes).to_bytes()),
    )?;
    fs::write("registry/dist/index.json", bytes)?;
    println!("Packaged and signed provider plugins.");
    Ok(())
}
/// Scaffolds `plugins/<id>`: a manifest, plus a Rust plugin crate for managed providers.
fn new_provider(id: &str, declarative: bool) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !id.starts_with('-'),
        "Provider IDs use lowercase letters, digits and hyphens"
    );
    let dir = Path::new("plugins").join(id);
    ensure!(!dir.exists(), "plugins/{id} already exists");
    let name: String = id
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let env = format!("{}_API_KEY", id.to_uppercase().replace('-', "_"));
    let host = format!("api.{id}.com");
    let (kind, managed) = if declarative {
        ("declarative", String::new())
    } else {
        ("managed", MANAGED_FIELDS.replace("{name}", &name))
    };
    let manifest = MANIFEST_TEMPLATE
        .replace("{id}", id)
        .replace("{name}", &name)
        .replace("{kind}", kind)
        .replace("{managed}", &managed)
        .replace("{host}", &host)
        .replace("{env}", &env);
    let parsed: Manifest = serde_json::from_str(&manifest)?;
    getkeys_core::plugins::validate_manifest(&parsed)?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("manifest.json"), manifest)?;
    if declarative {
        println!("Created plugins/{id}/manifest.json. See docs/providers.md.");
        return Ok(());
    }
    fs::write(
        dir.join("Cargo.toml"),
        fs::read_to_string("plugins/github/Cargo.toml")?
            .replace("getkeys-plugin-github", &format!("getkeys-plugin-{id}")),
    )?;
    fs::create_dir_all(dir.join("src"))?;
    fs::write(
        dir.join("src/lib.rs"),
        PLUGIN_TEMPLATE
            .replace("{name}", &name)
            .replace("{host}", &host),
    )?;
    // Best effort: the template's line breaks depend on the provider name's length.
    let _ = std::process::Command::new("rustfmt")
        .args(["--edition", "2024"])
        .arg(dir.join("src/lib.rs"))
        .status();
    let mut cargo = fs::read_to_string("Cargo.toml")?;
    let start = cargo
        .find("\nmembers = [")
        .context("Workspace members not found in Cargo.toml")?;
    let end = start
        + cargo[start..]
            .find(']')
            .context("Workspace members are malformed")?;
    cargo.insert_str(end, &format!(", \"plugins/{id}\""));
    fs::write("Cargo.toml", cargo)?;
    println!(
        "Created plugins/{id} and added it to the workspace. Run cargo test -p getkeys-plugin-{id}, then npm run plugins:build. See docs/providers.md."
    );
    Ok(())
}
const MANIFEST_TEMPLATE: &str = r#"{
  "id": "official.{id}",
  "name": "{name}",
  "publisher": "getkeys",
  "version": "0.1.0",
  "description": "Keep {name} API keys and check that they still work.",
  "homepage": "https://{id}.com",
  "kind": "{kind}",
  "category": "ai",
  "hosts": ["{host}"],{managed}
  "credential_fields": [
    {
      "key": "token",
      "label": "API key",
      "placeholder": "Paste the key",
      "required": true,
      "env": "{env}"
    }
  ],
  "token_page": "https://{id}.com/settings/api-keys",
  "key_pattern": "",
  "validate": { "url": "https://{host}/v1/models" },
  "sources": [{ "kind": "env", "var": "{env}", "role": "key" }]
}
"#;
const MANAGED_FIELDS: &str = r#"
  "operations": ["identity", "validate_credential"],
  "account_fields": [
    {
      "key": "token",
      "label": "{name} management key",
      "placeholder": "Paste the key",
      "required": true
    }
  ],"#;
const PLUGIN_TEMPLATE: &str = r#"use getkeys_plugin_api::*;
use serde_json::Value;
pub const MANIFEST: &str = include_str!("../manifest.json");

pub fn handle(call: Call) -> Step {
    execute(call)
        .unwrap_or_else(|_| error("invalid_response", "{name} returned an unexpected response."))
}

fn execute(call: Call) -> Result<Step, &'static str> {
    if let Some(response) = &call.response {
        if let Some(error) = provider_error(response) {
            return Ok(error);
        }
        if matches!(call.operation, Operation::ValidateCredential { .. }) {
            return Ok(done(Outcome::Done));
        }
        let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
        return Ok(match call.operation {
            Operation::Identity => done(Outcome::Identity(Identity {
                id: v["id"].as_str().ok_or("id")?.into(),
                name: v["name"].as_str().ok_or("name")?.into(),
            })),
            _ => error("unsupported", "This {name} operation is unavailable."),
        });
    }
    let req = match call.operation {
        Operation::Identity => get("https://{host}/v1/me".into()),
        Operation::ValidateCredential { .. } => {
            let mut r = get("https://{host}/v1/models".into());
            r.auth = RequestAuth::Candidate;
            r
        }
        _ => return Ok(error("unsupported", "This {name} operation is unavailable.")),
    };
    Ok(request(req, Value::Null))
}
#[cfg(target_arch = "wasm32")]
#[extism_pdk::plugin_fn]
pub fn describe() -> extism_pdk::FnResult<String> {
    Ok(MANIFEST.into())
}
#[cfg(target_arch = "wasm32")]
#[extism_pdk::plugin_fn]
pub fn run(call: extism_pdk::Json<Call>) -> extism_pdk::FnResult<extism_pdk::Json<Step>> {
    Ok(extism_pdk::Json(handle(call.0)))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;
    #[test]
    fn identity_reads_the_account() {
        let step = handle(Call {
            operation: Operation::Identity,
            config: BTreeMap::new(),
            cursor: Value::Null,
            response: Some(HttpResponse {
                status: 200,
                body: json!({"id": "acct_1", "name": "Ada"}).to_string(),
                headers: BTreeMap::new(),
            }),
        });
        assert!(matches!(step, Step::Complete { result: Outcome::Identity(i) } if i.id == "acct_1"));
    }
}
"#;
fn types() -> Result<()> {
    let cfg = ts_rs::Config::default().with_large_int("number");
    let declarations = [
        Manifest::decl(&cfg),
        ProviderKind::decl(&cfg),
        AuthMethod::decl(&cfg),
        Validate::decl(&cfg),
        Source::decl(&cfg),
        SourceRole::decl(&cfg),
        SourceKind::decl(&cfg),
        FileFormat::decl(&cfg),
        ConfigField::decl(&cfg),
        NewResource::decl(&cfg),
        ResourceOption::decl(&cfg),
        Choice::decl(&cfg),
        OperationKind::decl(&cfg),
        Resource::decl(&cfg),
        Owner::decl(&cfg),
        Created::decl(&cfg),
        PluginView::decl(&cfg),
        ConnectionView::decl(&cfg),
        Discovery::decl(&cfg),
        Deletion::decl(&cfg),
        ConnectionState::decl(&cfg),
        CredentialView::decl(&cfg),
        KeyField::decl(&cfg),
        Rotation::decl(&cfg),
        RotationState::decl(&cfg),
        Activity::decl(&cfg),
        Snapshot::decl(&cfg),
        AuthProgress::decl(&cfg),
        RoleChoice::decl(&cfg),
        ImportInput::decl(&cfg),
        EnvLine::decl(&cfg),
        CustomField::decl(&cfg),
        Prefs::decl(&cfg),
        AgentClient::decl(&cfg),
        AgentInfo::decl(&cfg),
        Command::decl(&cfg),
    ];
    let mut output = String::from("// Generated by cargo run -p xtask -- types.\n");
    for declaration in declarations {
        output.push_str("export ");
        output.push_str(&declaration);
        output.push('\n');
    }
    fs::create_dir_all("apps/desktop/src")?;
    fs::write("apps/desktop/src/types.ts", output)?;
    Ok(())
}
