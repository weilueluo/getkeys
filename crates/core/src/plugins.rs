use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use getkeys_plugin_api::{Call, Manifest, Outcome, ProviderKind, Step};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

pub const TRUSTED_KEY: &str = include_str!("../../../registry/public-key.hex");
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub manifest: Manifest,
    /// The provider's code; empty for declarative providers.
    #[serde(default)]
    pub wasm: String,
    /// Digest of the code, or of the manifest JSON for declarative providers.
    pub sha256: String,
}
impl Package {
    pub fn declarative(&self) -> bool {
        self.manifest.kind == ProviderKind::Declarative
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub packages: Vec<Package>,
}

#[derive(Clone)]
pub struct Registry {
    pub catalog: Catalog,
    pub source: PathBuf,
    pub installed: PathBuf,
    key: String,
}

pub fn verify_catalog(bytes: &[u8], signature: &str, key: &str) -> Result<Catalog> {
    let raw: [u8; 32] = hex::decode(key.trim())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid registry public key"))?;
    let key = VerifyingKey::from_bytes(&raw)?;
    let sig = Signature::from_slice(&STANDARD.decode(signature.trim())?)?;
    key.verify_strict(bytes, &sig)
        .context("Marketplace signature is invalid")?;
    let catalog: Catalog = serde_json::from_slice(bytes)?;
    let mut ids = std::collections::HashSet::new();
    for p in &catalog.packages {
        validate_manifest(&p.manifest)?;
        ensure!(
            ids.insert(&p.manifest.id),
            "Duplicate provider in the catalog"
        );
        ensure!(
            if p.declarative() {
                p.wasm.is_empty()
            } else {
                safe_name(&p.wasm) && p.wasm.ends_with(".wasm")
            },
            "Invalid provider package path"
        );
        ensure!(
            p.sha256.len() == 64 && p.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid artifact digest"
        );
    }
    Ok(catalog)
}
fn safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 160
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && !s.contains("..")
}
pub fn validate_manifest(m: &Manifest) -> Result<()> {
    ensure!(
        safe_name(&m.id) && safe_name(&m.version),
        "Invalid provider identity or version"
    );
    ensure!(!m.hosts.is_empty(), "Provider must declare its hosts");
    for host in &m.hosts {
        let name = host.strip_prefix("*.").unwrap_or(host);
        ensure!(
            name.contains('.')
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-".contains(&b))
                && !name.contains(".."),
            "Invalid provider host declaration"
        );
    }
    ensure!(
        m.kind == ProviderKind::Managed || m.operations.is_empty(),
        "A declarative provider cannot declare operations"
    );
    if !m.key_pattern.is_empty() {
        regex::Regex::new(&m.key_pattern).context("Invalid provider key pattern")?;
    }
    if let Some(v) = &m.validate {
        crate::http::validate_url(&v.url, &m.hosts).context("Invalid provider check URL")?;
    }
    for page in [&m.token_page, &m.key_page] {
        ensure!(
            page.is_empty() || page.starts_with("https://"),
            "Provider pages must use HTTPS"
        );
    }
    let mut sources = std::collections::HashSet::new();
    for s in &m.sources {
        ensure!(
            sources.insert(crate::discover::source_id(s)),
            "Duplicate provider source"
        );
    }
    Ok(())
}

/// The bytes a package's digest covers.
pub fn manifest_bytes(m: &Manifest) -> Vec<u8> {
    serde_json::to_vec(m).unwrap_or_default()
}

impl Registry {
    pub fn open(source: PathBuf, installed: PathBuf) -> Result<Self> {
        Self::with_key(source, installed, TRUSTED_KEY)
    }
    pub fn with_key(source: PathBuf, installed: PathBuf, key: &str) -> Result<Self> {
        let bytes = std::fs::read(source.join("index.json"))
            .context("Marketplace packages are missing. Run npm run plugins:build.")?;
        let catalog = verify_catalog(
            &bytes,
            &std::fs::read_to_string(source.join("index.sig"))?,
            key,
        )?;
        std::fs::create_dir_all(&installed)?;
        Ok(Self {
            catalog,
            source,
            installed,
            key: key.into(),
        })
    }
    pub fn package(&self, id: &str) -> Result<&Package> {
        self.catalog
            .packages
            .iter()
            .find(|p| p.manifest.id == id)
            .context("Provider was not found in the verified catalog")
    }
    pub fn installed_package(&self, id: &str) -> Result<Option<Package>> {
        ensure!(safe_name(id), "Invalid provider ID");
        let pointer = self.installed.join(format!("{id}.json"));
        if !pointer.exists() {
            return Ok(None);
        }
        // The complete signed catalog is retained so installed versions remain verifiable after an update.
        let verified = || -> Result<Package> {
            let saved: Installed = serde_json::from_slice(&std::fs::read(&pointer)?)?;
            let catalog = verify_catalog(saved.catalog.as_bytes(), &saved.signature, &self.key)?;
            catalog
                .packages
                .into_iter()
                .find(|p| p.manifest.id == id)
                .context("Installed provider is absent from its signed catalog")
        };
        match verified() {
            Ok(package) => Ok(Some(package)),
            Err(_) => {
                // Installed from a catalog this GetKeys no longer reads: replace it with the
                // current package so its accounts keep working.
                std::fs::remove_file(&pointer)?;
                if self.package(id).is_err() {
                    return Ok(None);
                }
                self.install(id, true)?;
                verified().map(Some)
            }
        }
    }
    pub fn install(&self, id: &str, accept_permissions: bool) -> Result<()> {
        let package = self.package(id)?;
        if let Some(old) = self.installed_package(id)? {
            let broadened = package
                .manifest
                .hosts
                .iter()
                .any(|h| !old.manifest.hosts.contains(h))
                || package
                    .manifest
                    .operations
                    .iter()
                    .any(|o| !old.manifest.operations.contains(o))
                || old.manifest.auth != package.manifest.auth;
            ensure!(
                !broadened || accept_permissions,
                "This update requests additional permissions. Review and approve the provider update."
            );
        }
        if package.declarative() {
            verify_artifact(package, &manifest_bytes(&package.manifest))?;
        } else {
            let bytes = std::fs::read(self.source.join(&package.wasm))?;
            verify_artifact(package, &bytes)?;
            let mut runtime = Runtime::new(package.clone(), &bytes)?;
            let actual: Manifest =
                serde_json::from_str(runtime.plugin.call::<_, String>("describe", "")?.as_str())?;
            ensure!(
                actual == package.manifest,
                "Provider code and catalog manifest disagree"
            );
            let artifact = self.installed.join(format!("{}.wasm", package.sha256));
            if !artifact.exists() {
                std::fs::write(&artifact, &bytes)?;
            }
        }
        let saved = Installed {
            catalog: std::fs::read_to_string(self.source.join("index.json"))?,
            signature: std::fs::read_to_string(self.source.join("index.sig"))?,
        };
        let temp = self.installed.join(format!("{}.tmp", crate::new_id()?));
        std::fs::write(&temp, serde_json::to_vec(&saved)?)?;
        let target = self.installed.join(format!("{id}.json"));
        // Windows rename does not replace existing files; only the metadata pointer is replaced.
        if target.exists() {
            std::fs::remove_file(&target)?;
        }
        std::fs::rename(temp, target)?;
        Ok(())
    }
    pub fn load(&self, id: &str) -> Result<Runtime> {
        let p = self
            .installed_package(id)?
            .context("Add this provider first")?;
        ensure!(
            !p.declarative(),
            "This provider has no account features; its keys are listed on their own"
        );
        let bytes = std::fs::read(self.installed.join(format!("{}.wasm", p.sha256)))?;
        verify_artifact(&p, &bytes)?;
        Runtime::new(p, &bytes)
    }
    pub fn uninstall(&self, id: &str) -> Result<()> {
        ensure!(safe_name(id), "Invalid provider ID");
        std::fs::remove_file(self.installed.join(format!("{id}.json")))?;
        Ok(())
    }
    pub fn download(url: &str, path: &Path) -> Result<()> {
        let base = url::Url::parse(url)?;
        ensure!(
            base.scheme() == "https" && base.username().is_empty() && base.password().is_none(),
            "Marketplace URL must use HTTPS"
        );
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()?;
        let fetch = |name: &str| -> Result<Vec<u8>> {
            let response = client.get(base.join(name)?).send()?.error_for_status()?;
            crate::http::bounded_response(response, 16 * 1024 * 1024)
        };
        let index = fetch("index.json")?;
        let signature = String::from_utf8(fetch("index.sig")?)?;
        let catalog = verify_catalog(&index, &signature, TRUSTED_KEY)?;
        let staging = path.with_extension(crate::new_id()?);
        std::fs::create_dir_all(&staging)?;
        for package in catalog.packages.iter().filter(|p| !p.declarative()) {
            let bytes = fetch(&package.wasm)?;
            verify_artifact(package, &bytes)?;
            std::fs::write(staging.join(&package.wasm), bytes)?;
        }
        std::fs::write(staging.join("index.json"), index)?;
        std::fs::write(staging.join("index.sig"), signature)?;
        ensure!(
            !path.exists(),
            "Choose a new catalog directory so an existing verified catalog is not overwritten"
        );
        std::fs::rename(staging, path)?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Installed {
    catalog: String,
    signature: String,
}
pub fn verify_artifact(p: &Package, bytes: &[u8]) -> Result<()> {
    ensure!(
        hex::encode(Sha256::digest(bytes)) == p.sha256,
        "Provider package failed integrity verification"
    );
    Ok(())
}

pub struct Runtime {
    pub package: Package,
    plugin: extism::Plugin,
}
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct OperationError {
    pub message: String,
    pub may_have_changed: bool,
}
impl Runtime {
    pub fn new(package: Package, bytes: &[u8]) -> Result<Self> {
        let manifest = extism::Manifest::new([extism::Wasm::data(bytes.to_vec())])
            .with_memory_max(1024)
            .with_timeout(Duration::from_secs(5))
            .with_allowed_hosts(std::iter::empty());
        let plugin = extism::Plugin::new(manifest, [], false).context("Unable to load provider")?;
        Ok(Self { package, plugin })
    }
    pub fn execute(
        &mut self,
        operation: getkeys_plugin_api::Operation,
        config: BTreeMap<String, String>,
        mut send: impl FnMut(
            getkeys_plugin_api::HttpRequest,
        ) -> Result<getkeys_plugin_api::HttpResponse>,
    ) -> Result<Outcome> {
        let mut may_have_changed = false;
        self.execute_inner(operation, config, &mut send, &mut may_have_changed)
            .map_err(|error| {
                OperationError {
                    message: format!("{error:#}"),
                    may_have_changed,
                }
                .into()
            })
    }
    fn execute_inner(
        &mut self,
        operation: getkeys_plugin_api::Operation,
        config: BTreeMap<String, String>,
        send: &mut impl FnMut(
            getkeys_plugin_api::HttpRequest,
        ) -> Result<getkeys_plugin_api::HttpResponse>,
        may_have_changed: &mut bool,
    ) -> Result<Outcome> {
        ensure!(
            self.package.manifest.operations.contains(&operation.kind()),
            "This provider does not support the requested operation"
        );
        let mut call = Call {
            operation,
            config,
            cursor: serde_json::Value::Null,
            response: None,
        };
        for _ in 0..100 {
            let output = self
                .plugin
                .call::<_, String>("run", serde_json::to_string(&call)?)
                .map_err(|_| {
                    anyhow::anyhow!("Provider code failed or exceeded its resource limit")
                })?;
            let step: Step =
                serde_json::from_str(&output).context("Provider returned an invalid response")?;
            match step {
                Step::Request { request, cursor } => {
                    crate::http::validate_url(&request.url, &self.package.manifest.hosts)?;
                    let previous = *may_have_changed;
                    let aws_read = request.form.get("Action").is_some_and(|a| {
                        a.starts_with("List") || a.starts_with("Get") || a.starts_with("Describe")
                    });
                    if request.method != "GET" && !aws_read {
                        *may_have_changed = true;
                    }
                    let response = send(request)?;
                    if (400..500).contains(&response.status) {
                        *may_have_changed = previous;
                    }
                    call.response = Some(response);
                    call.cursor = cursor;
                }
                Step::Complete { result } => return Ok(result),
                Step::Error { code, message } => bail!("{code}: {message}"),
            }
        }
        bail!("Provider exceeded the request limit; no further requests were issued")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    #[test]
    fn signatures_bind_the_entire_catalog() {
        let key = SigningKey::from_bytes(&[3; 32]);
        let bytes = br#"{"packages":[]}"#;
        let sig = STANDARD.encode(key.sign(bytes).to_bytes());
        let public = hex::encode(key.verifying_key().as_bytes());
        assert!(verify_catalog(bytes, &sig, &public).is_ok());
        assert!(verify_catalog(br#"{"packages": []}"#, &sig, &public).is_err());
    }
    #[test]
    fn rejects_artifact_traversal() {
        assert!(!safe_name("../evil.wasm"));
        assert!(!safe_name("C:\\evil.wasm"));
    }
    #[test]
    fn installs_declarative_providers_without_code() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "id": "test.acme", "name": "Acme", "publisher": "t", "version": "1", "description": "",
            "homepage": "https://acme.dev", "kind": "declarative", "hosts": ["api.acme.dev"],
            "credential_fields": [], "key_pattern": "^acme_[a-z]+$",
            "validate": {"url": "https://api.acme.dev/me"},
            "sources": [{"kind": "env", "var": "ACME_KEY", "role": "key"}]
        }))
        .unwrap();
        let sha256 = hex::encode(Sha256::digest(manifest_bytes(&manifest)));
        let catalog = |sha256: &str| {
            serde_json::to_vec(&Catalog {
                packages: vec![Package {
                    manifest: manifest.clone(),
                    wasm: String::new(),
                    sha256: sha256.into(),
                }],
            })
            .unwrap()
        };
        let dir = tempfile::tempdir().unwrap();
        let write = |bytes: &[u8]| {
            std::fs::write(dir.path().join("index.json"), bytes).unwrap();
            std::fs::write(
                dir.path().join("index.sig"),
                STANDARD.encode(key.sign(bytes).to_bytes()),
            )
            .unwrap();
        };
        let public = hex::encode(key.verifying_key().as_bytes());
        // A digest that doesn't cover the manifest is refused.
        write(&catalog(&"0".repeat(64)));
        let bad =
            Registry::with_key(dir.path().into(), dir.path().join("installed"), &public).unwrap();
        assert!(bad.install("test.acme", false).is_err());
        write(&catalog(&sha256));
        let registry =
            Registry::with_key(dir.path().into(), dir.path().join("installed"), &public).unwrap();
        registry.install("test.acme", false).unwrap();
        assert!(registry.installed_package("test.acme").unwrap().is_some());
        assert!(registry.load("test.acme").is_err());
        // An install from a catalog this build can't read is replaced silently.
        std::fs::write(dir.path().join("installed").join("test.acme.json"), "{}").unwrap();
        assert!(registry.installed_package("test.acme").unwrap().is_some());
    }
    #[test]
    fn rejects_unsafe_manifests() {
        let base = serde_json::json!({
            "id": "test.acme", "name": "Acme", "publisher": "t", "version": "1", "description": "",
            "homepage": "https://acme.dev", "kind": "declarative", "hosts": ["api.acme.dev"],
            "credential_fields": []
        });
        let with = |k: &str, v: serde_json::Value| {
            let mut m = base.clone();
            m[k] = v;
            validate_manifest(&serde_json::from_value(m).unwrap())
        };
        assert!(
            with(
                "validate",
                serde_json::json!({"url": "https://evil.dev/me"})
            )
            .is_err()
        );
        assert!(with("key_pattern", serde_json::json!("(")).is_err());
        assert!(with("operations", serde_json::json!(["identity"])).is_err());
        assert!(
            with(
                "sources",
                serde_json::json!([{"kind":"env","var":"A"},{"kind":"env","var":"A"}])
            )
            .is_err()
        );
    }
}
