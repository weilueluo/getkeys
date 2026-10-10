use anyhow::{Context, Result, bail, ensure};
use getkeys_core::{App, api::*, model::*, new_id, plugins::Registry, vault::Vault};
use std::{collections::BTreeMap, path::PathBuf};

const REQUIRED: &[&str] = &[
    "GETKEYS_LIVE_GITHUB_TOKEN",
    "GETKEYS_LIVE_AWS_ACCESS_KEY_ID",
    "GETKEYS_LIVE_AWS_SECRET_ACCESS_KEY",
    "GETKEYS_LIVE_AWS_USER",
    "GETKEYS_LIVE_SUPABASE_TOKEN",
    "GETKEYS_LIVE_SUPABASE_PROJECT",
    "GETKEYS_LIVE_VERCEL_TOKEN",
];
fn env(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("Missing {name}"))
}
pub fn run() -> Result<()> {
    let missing: Vec<_> = REQUIRED
        .iter()
        .filter(|name| std::env::var(name).map_or(true, |v| v.trim().is_empty()))
        .collect();
    ensure!(
        missing.is_empty(),
        "Live tests are BLOCKED by missing sandbox configuration: {}. See docs/live-tests.md. No provider requests were made.",
        missing.iter().map(|s| **s).collect::<Vec<_>>().join(", ")
    );
    ensure!(
        std::env::var("GETKEYS_LIVE_ALLOW_WRITES").as_deref() == Ok("true"),
        "Set GETKEYS_LIVE_ALLOW_WRITES=true to authorize temporary keys in the configured sandbox accounts"
    );
    let aws_user = env("GETKEYS_LIVE_AWS_USER")?;
    ensure!(
        aws_user.starts_with("getkeys-test-"),
        "Use a dedicated IAM user named getkeys-test-* with no existing access keys"
    );
    let dir = tempfile::tempdir()?;
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
    let app = App {
        vault: Vault::open(&dir.path().join("vault.db"), &key)?,
        registry: Registry::open(PathBuf::from("registry/dist"), dir.path().join("plugins"))?,
    };
    for provider in ["github", "aws", "vercel", "supabase"] {
        let plugin = format!("official.{provider}");
        app.install(&plugin, false)?;
        let auth = if provider == "aws" {
            AuthMaterial::Aws {
                access_key_id: env("GETKEYS_LIVE_AWS_ACCESS_KEY_ID")?,
                secret_access_key: env("GETKEYS_LIVE_AWS_SECRET_ACCESS_KEY")?,
                session_token: std::env::var("GETKEYS_LIVE_AWS_SESSION_TOKEN").ok(),
                expires_at: None,
                sso: None,
            }
        } else {
            AuthMaterial::Bearer {
                token: env(&format!("GETKEYS_LIVE_{}_TOKEN", provider.to_uppercase()))?,
                refresh_token: None,
                expires_at: None,
                broker_registration: None,
            }
        };
        let mut config = BTreeMap::new();
        if provider == "vercel"
            && let Ok(team) = env("GETKEYS_LIVE_VERCEL_TEAM_ID")
        {
            config.insert("team_id".into(), team);
        }
        let mut c = Connection {
            view: ConnectionView {
                id: provider.into(),
                plugin_id: plugin,
                name: "Live sandbox".into(),
                state: ConnectionState::Connected,
                config,
                source: None,
            },
            identity_id: "live-test".into(),
            linked: Vec::new(),
            dedicated: None,
            auth,
        };
        app.vault.put("connection", provider, &c)?;
        let Outcome::Identity(identity) = app.execute(provider, Operation::Identity, None, None)?
        else {
            bail!("{provider}: identity response was invalid")
        };
        c.identity_id = identity.id;
        app.vault.put("connection", provider, &c)?;
    }
    let mut errors = Vec::new();
    for (name, result) in [
        ("AWS key lifecycle", key_lifecycle(&app, "aws", &aws_user)),
        (
            "Supabase key lifecycle",
            key_lifecycle(&app, "supabase", &env("GETKEYS_LIVE_SUPABASE_PROJECT")?),
        ),
        ("Vercel key lifecycle", key_lifecycle(&app, "vercel", "")),
    ] {
        match result {
            Ok(()) => println!("PASS {name}"),
            Err(error) => {
                eprintln!("FAIL {name}: {error:#}");
                errors.push(name);
            }
        }
    }
    ensure!(
        errors.is_empty(),
        "Live provider checks failed: {}",
        errors.join(", ")
    );
    println!(
        "All provider API checks passed. Browser consent and deployment health still require the manual checks in docs/live-tests.md."
    );
    Ok(())
}
fn key_lifecycle(app: &App, provider: &str, resource: &str) -> Result<()> {
    let initial = app.sync(provider, resource)?;
    if provider == "aws" {
        ensure!(
            initial.is_empty(),
            "AWS sandbox user must have no access keys before this test"
        );
    }
    let mut created = Vec::new();
    let result = (|| -> Result<()> {
        for _ in 0..2 {
            let c = app.create(provider, resource, &format!("getkeys-live-{}", new_id()?))?;
            created.push(c.clone());
            let secret = app.secret(&c.id)?;
            app.execute(
                provider,
                Operation::ValidateCredential {
                    resource: resource.into(),
                },
                Some(&secret),
                None,
            )?;
            ensure!(
                app.sync(provider, resource)?
                    .iter()
                    .any(|key| key.provider_id == c.provider_id),
                "Created key was absent from inventory"
            );
        }
        if provider == "aws" {
            ensure!(
                app.create(provider, resource, "third-key-must-fail")
                    .is_err(),
                "AWS two-key preflight did not reject a third key"
            );
            app.execute(
                provider,
                Operation::DeactivateCredential {
                    resource: resource.into(),
                    credential: created[0].provider_id.clone(),
                },
                None,
                None,
            )?;
        } else {
            let Outcome::Credential(read) = app.execute(
                provider,
                Operation::ReadCredential {
                    resource: resource.into(),
                    credential: created[0].provider_id.clone(),
                },
                None,
                None,
            )?
            else {
                bail!("Supabase did not reveal its key")
            };
            ensure!(read.value.is_some(), "Supabase reveal returned no value");
        }
        Ok(())
    })();
    let mut cleanup = Vec::new();
    for c in created {
        if let Err(error) = app.execute(
            provider,
            Operation::RevokeCredential {
                resource: resource.into(),
                credential: c.provider_id.clone(),
            },
            None,
            None,
        ) {
            cleanup.push(format!("{}: {error}", c.provider_id));
        }
    }
    ensure!(
        cleanup.is_empty(),
        "Cleanup failed; remove these sandbox keys manually: {}",
        cleanup.join(", ")
    );
    result
}
