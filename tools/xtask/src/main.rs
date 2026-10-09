use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use getkeys_core::{
    api::*,
    commands::Command,
    discover::Discovery,
    model::*,
    plugins::{Catalog, Package, verify_catalog},
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
                getkeys_core::plugins::verify_artifact(
                    &p,
                    &fs::read(Path::new("registry/dist").join(&p.wasm))?,
                )?;
            }
            println!("Marketplace signature and all plugin artifacts verified.");
        }
        Some("types") => types()?,
        _ => bail!("Usage: cargo run -p xtask -- <init-signing|package|verify|types|live>"),
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
            serde_json::from_slice(&fs::read(format!("plugins/{id}/manifest.json"))?)?;
        let bytes = fs::read(format!(
            "target/wasm32-unknown-unknown/release/getkeys_plugin_{id}.wasm"
        ))?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let wasm = format!("{}-{}.wasm", manifest.id, manifest.version);
        fs::write(Path::new("registry/dist").join(&wasm), bytes)?;
        packages.push(Package {
            manifest,
            wasm,
            sha256,
        });
    }
    let bytes = serde_json::to_vec_pretty(&Catalog {
        version: 1,
        packages,
    })?;
    fs::write(
        "registry/dist/index.sig",
        STANDARD.encode(key.sign(&bytes).to_bytes()),
    )?;
    fs::write("registry/dist/index.json", bytes)?;
    println!("Packaged and signed provider plugins.");
    Ok(())
}
fn types() -> Result<()> {
    let cfg = ts_rs::Config::default().with_large_int("number");
    let declarations = [
        Manifest::decl(&cfg),
        AuthMethod::decl(&cfg),
        ConfigField::decl(&cfg),
        OperationKind::decl(&cfg),
        Resource::decl(&cfg),
        PluginView::decl(&cfg),
        ConnectionView::decl(&cfg),
        Discovery::decl(&cfg),
        Deletion::decl(&cfg),
        ConnectionState::decl(&cfg),
        CredentialView::decl(&cfg),
        KeyField::decl(&cfg),
        Binding::decl(&cfg),
        Destination::decl(&cfg),
        Rotation::decl(&cfg),
        RotationState::decl(&cfg),
        Activity::decl(&cfg),
        Snapshot::decl(&cfg),
        AuthProgress::decl(&cfg),
        RoleChoice::decl(&cfg),
        ImportInput::decl(&cfg),
        EnvLine::decl(&cfg),
        CustomField::decl(&cfg),
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
