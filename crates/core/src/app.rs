use crate::{
    api::*,
    auth::{self, PollResult},
    discover::{self, Discovery, Found},
    http::{Transport, auth_from_secret},
    model::*,
    new_id,
    plugins::Registry,
    vault::Vault,
};
use anyhow::{Context, Result, bail, ensure};
use std::{collections::BTreeMap, path::PathBuf};

pub struct App {
    pub vault: Vault,
    pub registry: Registry,
}

impl App {
    pub fn open(data: PathBuf, registry: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data)?;
        let app = Self {
            vault: Vault::system(&data.join("vault.db"))?,
            registry: Registry::open(registry, data.join("plugins"))?,
        };
        app.apply_safe_updates();
        Ok(app)
    }
    /// A second handle on the same vault and providers, for work that runs alongside others.
    pub fn fork(&self) -> Result<Self> {
        Ok(Self {
            vault: self.vault.fork()?,
            registry: self.registry.clone(),
        })
    }
    /// Updates that request no new hosts, operations, or sign-in method are applied silently;
    /// the rest wait for approval in Providers.
    fn apply_safe_updates(&self) {
        for p in &self.registry.catalog.packages {
            let id = &p.manifest.id;
            if let Ok(Some(installed)) = self.registry.installed_package(id)
                && installed.sha256 != p.sha256
                && self.ensure_plugin_idle(id).is_ok()
            {
                let _ = self.registry.install(id, false);
            }
        }
    }
    pub fn data_dir() -> Result<PathBuf> {
        Ok(directories::ProjectDirs::from("dev", "getkeys", "GetKeys")
            .context("User data directory is unavailable")?
            .data_local_dir()
            .to_owned())
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let plugins = self
            .registry
            .catalog
            .packages
            .iter()
            .map(|p| {
                let installed = self.registry.installed_package(&p.manifest.id)?;
                Ok(PluginView {
                    // What the installed code can do, until an update is approved.
                    manifest: installed
                        .as_ref()
                        .map_or_else(|| p.manifest.clone(), |i| i.manifest.clone()),
                    installed: installed.is_some(),
                    update_available: installed.is_some_and(|i| i.sha256 != p.sha256),
                    browser_sign_in: auth::browser_ready(&p.manifest.auth),
                })
            })
            .collect::<Result<_>>()?;
        let mut activity = self.vault.list::<Activity>("activity")?;
        activity.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
        activity.truncate(100);
        let connections = self.vault.list::<Connection>("connection")?;
        let manifests: BTreeMap<_, _> = connections
            .iter()
            .filter_map(|c| Some((c.view.id.clone(), self.manifest(&c.view.plugin_id).ok()?)))
            .collect();
        let now = time::OffsetDateTime::now_utc();
        Ok(Snapshot {
            plugins,
            credentials: self
                .vault
                .list::<Credential>("credential")?
                .into_iter()
                .map(|c| {
                    let fields = if !c.view.plugin_id.is_empty() {
                        self.manifest(&c.view.plugin_id)
                            .map_or_else(|_| custom_fields(&c), |m| key_fields(&m, &c))
                    } else if c.view.connection_id.is_empty() {
                        custom_fields(&c)
                    } else {
                        manifests
                            .get(&c.view.connection_id)
                            .map(|m| key_fields(m, &c))
                            .unwrap_or_default()
                    };
                    let connection = connections
                        .iter()
                        .find(|x| x.view.id == c.view.connection_id);
                    let problem = problem(&c, connection, now);
                    let origin = if c.view.origin.is_empty() {
                        legacy_origin(&c, connection).into()
                    } else {
                        c.view.origin.clone()
                    };
                    CredentialView {
                        fields,
                        problem,
                        origin,
                        ..c.view
                    }
                })
                .collect(),
            connections: connections.into_iter().map(|c| c.view).collect(),
            rotations: self.vault.rotations()?,
            activity,
            vault_path: self.vault.path().display().to_string(),
        })
    }
    /// Every provider's manifest, as installed when it is.
    pub fn manifests(&self) -> Vec<Manifest> {
        self.registry
            .catalog
            .packages
            .iter()
            .map(|p| {
                self.registry
                    .installed_package(&p.manifest.id)
                    .ok()
                    .flatten()
                    .map_or_else(|| p.manifest.clone(), |i| i.manifest)
            })
            .collect()
    }
    fn installed_ids(&self) -> Vec<String> {
        self.registry
            .catalog
            .packages
            .iter()
            .map(|p| p.manifest.id.clone())
            .filter(|id| {
                self.registry
                    .installed_package(id)
                    .is_ok_and(|p| p.is_some())
            })
            .collect()
    }
    pub fn manifest(&self, id: &str) -> Result<Manifest> {
        Ok(self
            .registry
            .installed_package(id)?
            .context("Add this provider first")?
            .manifest)
    }
    pub fn install(&self, id: &str, accept_permissions: bool) -> Result<()> {
        self.ensure_plugin_idle(id)?;
        self.registry.install(id, accept_permissions)?;
        self.vault.event("provider.added", id)
    }
    /// Removes a provider with its accounts and their keys. Keys stay valid at the provider,
    /// except the one GetKeys created for itself.
    pub fn uninstall(&self, id: &str) -> Result<()> {
        self.ensure_plugin_idle(id)?;
        let connections: Vec<Connection> = self
            .vault
            .list::<Connection>("connection")?
            .into_iter()
            .filter(|c| c.view.plugin_id == id)
            .collect();
        for c in &connections {
            if c.view.state != ConnectionState::Disconnected {
                self.disconnect(&c.view.id)?;
            }
        }
        let tx = self.vault.write()?;
        for c in self.vault.list::<Credential>("credential")? {
            let own = c.view.connection_id.is_empty() && c.view.plugin_id == id;
            if own
                || connections
                    .iter()
                    .any(|x| x.view.id == c.view.connection_id)
            {
                self.vault.remove("credential", &c.view.id)?;
            }
        }
        for c in &connections {
            self.vault.remove("connection", &c.view.id)?;
        }
        tx.commit()?;
        self.registry.uninstall(id)?;
        self.vault.event("provider.removed", id)
    }
    fn ensure_plugin_idle(&self, id: &str) -> Result<()> {
        let connections = self.vault.list::<Connection>("connection")?;
        ensure!(
            !self.vault.rotations()?.iter().any(|r| r.is_active()
                && connections
                    .iter()
                    .any(|c| r.uses_connection(&c.view.id) && c.view.plugin_id == id)),
            "Finish active rotations before changing this provider"
        );
        Ok(())
    }
    pub fn begin_auth(&self, id: &str, config: BTreeMap<String, String>) -> Result<AuthProgress> {
        let progress = auth::begin(&self.vault, &self.manifest(id)?, config, None)?;
        if let AuthProgress::Browser { url, .. } = &progress {
            open::that_detached(url).context("Unable to open the system browser")?;
        }
        Ok(progress)
    }
    pub fn reconnect(&self, id: &str) -> Result<AuthProgress> {
        let connection: Connection = self.vault.get("connection", id)?;
        match connection.view.source.as_deref() {
            Some("token") => bail!("Paste a new token to reconnect this account"),
            Some(source) => return self.connect_local(source, Some(id)),
            None => {}
        }
        let manifest = self.manifest(&connection.view.plugin_id)?;
        let config = connection
            .view
            .config
            .into_iter()
            .filter(|(key, _)| manifest.fields.iter().any(|f| f.key == *key))
            .collect();
        let progress = auth::begin(&self.vault, &manifest, config, Some(id))?;
        if let AuthProgress::Browser { url, .. } = &progress {
            open::that_detached(url).context("Unable to open the system browser")?;
        }
        Ok(progress)
    }
    pub fn poll_auth(&self, id: &str) -> Result<AuthProgress> {
        self.finish_auth(id, auth::poll(&self.vault, id)?)
    }
    pub fn choose_role(&self, id: &str, account: &str, role: &str) -> Result<AuthProgress> {
        self.finish_auth(id, auth::choose_role(&self.vault, id, account, role)?)
    }
    fn finish_auth(&self, id: &str, result: PollResult) -> Result<AuthProgress> {
        let PollResult::Authorized {
            plugin_id,
            config,
            auth,
        } = result
        else {
            let PollResult::Progress(progress) = result else {
                unreachable!()
            };
            return Ok(progress);
        };
        self.save_connection(id, plugin_id, config, auth, None)
    }
    /// Sessions already on this device that can be connected in one click.
    pub fn discover(&self) -> Result<Vec<Discovery>> {
        let connections = self.vault.list::<Connection>("connection")?;
        Ok(discover::scan(&self.manifests())
            .into_iter()
            .filter(|d| !d.key)
            .map(|mut d| {
                d.connected = connections.iter().any(|c| {
                    c.view.state != ConnectionState::Disconnected
                        && (c.view.source.as_deref() == Some(d.id.as_str())
                            || c.linked.contains(&d.id))
                });
                d
            })
            .collect())
    }
    pub fn connect_local(&self, source: &str, existing: Option<&str>) -> Result<AuthProgress> {
        let (plugin_id, found) = discover::load(&self.manifests(), source)?;
        self.ensure_installed(&plugin_id)?;
        let id = existing.map_or_else(new_id, |id| Ok(id.to_owned()))?;
        match found {
            Found::SignIn(config) => {
                let progress =
                    auth::begin(&self.vault, &self.manifest(&plugin_id)?, config, existing)?;
                if let AuthProgress::Browser { url, .. } = &progress {
                    open::that_detached(url).context("Unable to open the system browser")?;
                }
                Ok(progress)
            }
            Found::Material(auth) => {
                let fields = material_fields(&auth);
                match self.save_connection(
                    &id,
                    plugin_id.clone(),
                    BTreeMap::new(),
                    *auth,
                    Some(source.into()),
                ) {
                    // A sign-in the provider turns away is still a key found here; it waits
                    // in Others with the reason instead of failing the refresh.
                    Err(e) if refused(&e) || unauthorized(&e) => {
                        self.log(&format!("{source}: {e:#}"));
                        let mut key = self.save_found(
                            &plugin_id,
                            source,
                            &discover::sign_in_name(&self.manifests(), Some(source)),
                            SecretValue { fields },
                        )?;
                        let reason = if refused(&e) { CANT_SIGN_IN } else { REJECTED };
                        let mut c: Credential = self.vault.get("credential", &key.id)?;
                        c.view.problem = Some(reason.into());
                        self.vault.put("credential", &key.id, &c)?;
                        key.problem = c.view.problem;
                        Ok(AuthProgress::KeySaved {
                            credential: Box::new(key),
                        })
                    }
                    r => {
                        // Once it signs in, the turned-away copy has nothing left to say.
                        let stale = format!("found:{source}");
                        if r.is_ok()
                            && self
                                .vault
                                .get::<Credential>("credential", &stale)
                                .is_ok_and(|c| !c.view.retired && c.view.problem.is_some())
                        {
                            self.vault.remove("credential", &stale)?;
                        }
                        r
                    }
                }
            }
            Found::Key(value) => Ok(AuthProgress::KeySaved {
                credential: Box::new(self.save_found(
                    &plugin_id,
                    source,
                    &discover::sign_in_name(&self.manifests(), Some(source)),
                    value,
                )?),
            }),
        }
    }
    /// Keeps a key found on this device on its own, under its provider.
    fn save_found(
        &self,
        plugin_id: &str,
        source: &str,
        name: &str,
        value: SecretValue,
    ) -> Result<CredentialView> {
        let id = format!("found:{source}");
        let previous = self.vault.get::<Credential>("credential", &id).ok();
        ensure!(
            previous.as_ref().is_none_or(|p| !p.view.retired),
            "This key was deleted from GetKeys"
        );
        self.store_key(&id, plugin_id, name, value, previous, origin::FOUND)
    }
    fn store_key(
        &self,
        id: &str,
        plugin_id: &str,
        name: &str,
        value: SecretValue,
        previous: Option<Credential>,
        origin: &str,
    ) -> Result<CredentialView> {
        let manifest = self.manifest(plugin_id)?;
        let plain = |k: &str| {
            manifest
                .credential_fields
                .iter()
                .any(|f| f.key == k && !f.secret)
        };
        let (details, secrets): (BTreeMap<_, _>, BTreeMap<_, _>) =
            value.fields.into_iter().partition(|(k, _)| plain(k));
        ensure!(!secrets.is_empty(), "Enter the key's secret value");
        let created = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .ok();
        let c = Credential {
            view: CredentialView {
                id: id.into(),
                connection_id: String::new(),
                provider_id: "local".into(),
                name: name.trim().into(),
                credential_type: API_KEY.into(),
                resource: String::new(),
                project: String::new(),
                environment: String::new(),
                created_at: previous
                    .as_ref()
                    .map_or(created, |c| c.view.created_at.clone()),
                expires_at: None,
                value_available: true,
                can_rotate: false,
                retired: false,
                fields: Vec::new(),
                resource_name: String::new(),
                groups: previous
                    .as_ref()
                    .map(|c| c.view.groups.clone())
                    .unwrap_or_default(),
                problem: None,
                plugin_id: plugin_id.into(),
                origin: origin.into(),
            },
            value: Some(SecretValue { fields: secrets }),
            details,
        };
        self.vault.put("credential", id, &c)?;
        self.vault.event(
            if previous.is_some() {
                "credential.updated"
            } else {
                "credential.added"
            },
            id,
        )?;
        Ok(c.view)
    }
    /// Saves a key typed or pasted by the person under a provider, or replaces the value of an
    /// existing key after rotating it on the provider's page. Blank secret fields keep their
    /// value; a key the provider rejects is not saved.
    pub fn save_key(
        &self,
        id: Option<&str>,
        plugin_id: &str,
        name: &str,
        fields: BTreeMap<String, String>,
    ) -> Result<CredentialView> {
        let mut fields: BTreeMap<String, String> = fields
            .into_iter()
            .map(|(k, v)| (k, v.trim().to_owned()))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        let previous = match id {
            Some(id) => {
                let c: Credential = self.vault.get("credential", id)?;
                ensure!(
                    c.view.credential_type != SIGN_IN && c.view.credential_type != CUSTOM,
                    "Edit this key from its own form"
                );
                for (k, v) in c.value.iter().flat_map(|v| &v.fields).chain(&c.details) {
                    fields.entry(k.clone()).or_insert_with(|| v.clone());
                }
                Some(c)
            }
            None => None,
        };
        self.ensure_installed(plugin_id)?;
        let manifest = self.manifest(plugin_id)?;
        let name = if name.trim().is_empty() {
            previous
                .as_ref()
                .map_or_else(|| format!("{} key", manifest.name), |c| c.view.name.clone())
        } else {
            name.trim().to_owned()
        };
        for f in manifest.credential_fields.iter().filter(|f| f.required) {
            ensure!(fields.contains_key(&f.key), "Enter the {}", f.label);
        }
        let value = SecretValue { fields };
        let rejected = || {
            anyhow::anyhow!(
                "{} rejected this key. Check it and try again.",
                manifest.name
            )
        };
        match &previous {
            Some(c) if !c.view.connection_id.is_empty() => {
                // A key listed under an account keeps its place; only its value changes.
                let tx = self.vault.write()?;
                let mut c: Credential = self.vault.get("credential", &c.view.id)?;
                let (details, secrets): (BTreeMap<_, _>, BTreeMap<_, _>) = value
                    .fields
                    .into_iter()
                    .partition(|(k, _)| c.details.contains_key(k));
                c.value = Some(SecretValue { fields: secrets });
                c.details = details;
                c.view.name = name;
                c.view.value_available = true;
                c.view.problem = None;
                self.vault.put("credential", &c.view.id, &c)?;
                self.vault.event("credential.updated", &c.view.id)?;
                tx.commit()?;
                return Ok(c.view);
            }
            _ => {}
        }
        if crate::http::probe(&manifest, &value).ok().flatten() == Some(false) {
            return Err(rejected());
        }
        let tx = self.vault.write()?;
        let id = match &previous {
            // The place it was found still holds the old value, so the new one is kept apart
            // and the old place no longer brings the key back.
            Some(c) if c.view.id.starts_with("found:") => {
                crate::rotation::mark_retired(&self.vault, c)?;
                new_id()?
            }
            Some(c) => c.view.id.clone(),
            None => new_id()?,
        };
        let view = self.store_key(&id, plugin_id, &name, value, previous, origin::PASTED)?;
        tx.commit()?;
        Ok(view)
    }
    /// The provider's page for making a new key by hand, filled in with the key's name and the
    /// owner of the resource it is for, or the account's name.
    pub fn key_page(
        &self,
        plugin_id: &str,
        connection_id: Option<&str>,
        resource: &str,
        name: &str,
    ) -> Result<String> {
        let manifest = self
            .manifests()
            .into_iter()
            .find(|m| m.id == plugin_id)
            .context("Unknown provider")?;
        let template = if manifest.key_page.is_empty() {
            &manifest.token_page
        } else {
            &manifest.key_page
        };
        ensure!(
            !template.is_empty(),
            "{} has no page for making keys",
            manifest.name
        );
        let owner = match resource.split_once('/') {
            Some((owner, _)) => owner.to_owned(),
            None => connection_id
                .and_then(|id| self.vault.get::<Connection>("connection", id).ok())
                .map(|c| c.view.name)
                .unwrap_or_default(),
        };
        let encode =
            |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
        let filled = template
            .replace("{name}", &encode(name.trim()))
            .replace("{owner}", &encode(&owner));
        let Some((base, query)) = filled.split_once('?') else {
            return Ok(filled);
        };
        let kept: Vec<&str> = query
            .split('&')
            .filter(|p| p.split_once('=').is_none_or(|(_, v)| !v.is_empty()))
            .collect();
        Ok(if kept.is_empty() {
            base.to_owned()
        } else {
            format!("{base}?{}", kept.join("&"))
        })
    }
    /// Whether copied text looks like one of the provider's keys, to pick up a key made on its
    /// page from the clipboard.
    pub fn looks_like_key(&self, plugin_id: &str, text: &str) -> bool {
        let text = text.trim();
        let pattern = self
            .manifest(plugin_id)
            .map(|m| m.key_pattern)
            .ok()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| r"^[A-Za-z0-9_\-.:]{20,512}$".into());
        regex::Regex::new(&pattern).is_ok_and(|r| r.is_match(text))
    }
    /// Saves a key made on the provider's page and picked up from the clipboard, under the
    /// account it was made for when there is one. It joins `group` and, when it replaces an
    /// older key, that key's groups.
    #[allow(clippy::too_many_arguments)]
    pub fn save_captured(
        &self,
        plugin_id: &str,
        connection_id: Option<&str>,
        resource: &str,
        name: &str,
        value: &str,
        group: Option<&str>,
        replaces: Option<&str>,
    ) -> Result<CredentialView> {
        self.ensure_installed(plugin_id)?;
        let manifest = self.manifest(plugin_id)?;
        let field = manifest
            .credential_fields
            .iter()
            .find(|f| f.secret)
            .map_or("token", |f| f.key.as_str());
        let fields = BTreeMap::from([(field.to_owned(), value.trim().to_owned())]);
        let name = if name.trim().is_empty() {
            format!("{} key", manifest.name)
        } else {
            name.trim().to_owned()
        };
        let view = match connection_id.filter(|c| !c.is_empty()) {
            Some(connection_id) => self.import(ImportInput {
                connection_id: connection_id.into(),
                provider_id: None,
                resource: resource.into(),
                name,
                project: String::new(),
                environment: String::new(),
                fields,
            })?,
            None => self.save_key(None, plugin_id, &name, fields)?,
        };
        let mut c: Credential = self.vault.get("credential", &view.id)?;
        c.view.origin = origin::CAPTURED.into();
        let inherited = match replaces {
            Some(old) => self.vault.get::<Credential>("credential", old)?.view.groups,
            None => Vec::new(),
        };
        for g in inherited
            .into_iter()
            .chain(group.map(str::to_owned))
            .filter(|g| !g.trim().is_empty())
        {
            if !c.view.groups.contains(&g) {
                c.view.groups.push(g);
            }
        }
        self.vault.put("credential", &c.view.id, &c)?;
        Ok(c.view)
    }
    /// Replaces a key with a new one from its provider and retires the old one, keeping its
    /// groups: the provider's own roll when it has one, otherwise a new key is made and the old
    /// one revoked. Providers that can't make keys replace them on their page instead, through
    /// [`App::save_captured`].
    pub fn rotate_now(&self, id: &str) -> Result<CredentialView> {
        let old: Credential = self.vault.get("credential", id)?;
        ensure!(!old.view.retired, "This key was already replaced");
        ensure!(
            old.view.credential_type != SIGN_IN,
            "Replace the key an account signs in with from Providers"
        );
        ensure!(!old.view.connection_id.is_empty(), MAKE_ON_PAGE);
        let connection: Connection = self.vault.get("connection", &old.view.connection_id)?;
        let manifest = self.manifest(&connection.view.plugin_id)?;
        if manifest
            .operations
            .contains(&OperationKind::RotateCredential)
        {
            return self.roll(old);
        }
        ensure!(
            old.view.can_rotate
                && manifest
                    .operations
                    .contains(&OperationKind::CreateCredential)
                && manifest
                    .operations
                    .contains(&OperationKind::RevokeCredential),
            MAKE_ON_PAGE
        );
        // An unfinished rotation of this key carries on from where it stopped.
        let mut r = match self
            .vault
            .rotations()?
            .into_iter()
            .find(|r| r.is_active() && r.credential_id == id)
        {
            Some(r) => r,
            None => self.plan_rotation(id, &old.view.name)?,
        };
        if r.state == RotationState::Planned {
            r = self.advance_rotation(&r.id).inspect_err(|_| {
                if self
                    .vault
                    .rotation(&r.id)
                    .is_ok_and(|now| now.state == RotationState::Planned)
                {
                    let _ = self.abort_rotation(&r.id);
                }
            })?;
        }
        // A new key can take a few seconds to work everywhere, so checking it is retried
        // before the old one is retired.
        let mut attempt = 0;
        while matches!(
            r.state,
            RotationState::ReadyToRetire | RotationState::Deactivated
        ) {
            match self.advance_rotation(&r.id) {
                Ok(next) => r = next,
                Err(e) => {
                    attempt += 1;
                    if attempt > 5 || self.vault.rotation(&r.id)?.state != r.state {
                        return Err(e);
                    }
                    std::thread::sleep(std::time::Duration::from_secs(2 * attempt));
                }
            }
        }
        ensure!(
            r.state == RotationState::Complete,
            "The rotation needs attention. Open the key to finish it."
        );
        let mut c: Credential = self.vault.get(
            "credential",
            r.candidate_id
                .as_deref()
                .context("Replacement is missing")?,
        )?;
        c.view.groups = old.view.groups.clone();
        c.view.origin = origin::ROTATED.into();
        self.vault.put("credential", &c.view.id, &c)?;
        self.vault.event("credential.rotated", &c.view.id)?;
        Ok(c.view)
    }
    /// Has the provider replace a key in place with its own roll operation.
    fn roll(&self, old: Credential) -> Result<CredentialView> {
        let Outcome::Credential(remote) = self.execute(
            &old.view.connection_id,
            Operation::RotateCredential {
                resource: old.view.resource.clone(),
                credential: old.view.provider_id.clone(),
            },
            None,
            None,
        )?
        else {
            bail!("Provider did not return the new key")
        };
        let tx = self.vault.write()?;
        let replaced = remote.id != old.view.provider_id;
        let mut c = self.save_remote(&old.view.connection_id, remote)?;
        c.view.groups = old.view.groups.clone();
        c.view.problem = None;
        c.view.origin = origin::ROTATED.into();
        self.vault.put("credential", &c.view.id, &c)?;
        if replaced {
            crate::rotation::mark_retired(&self.vault, &old)?;
        }
        self.vault.event("credential.rotated", &c.view.id)?;
        tx.commit()?;
        Ok(c.view)
    }
    pub fn connect_token(
        &self,
        plugin_id: &str,
        fields: BTreeMap<String, String>,
        existing: Option<&str>,
    ) -> Result<AuthProgress> {
        let fields: BTreeMap<_, _> = fields
            .into_iter()
            .map(|(k, v)| (k, v.trim().to_owned()))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        ensure!(!fields.is_empty(), "Paste a token to connect");
        let auth = auth_from_secret(&SecretValue { fields })?;
        self.ensure_installed(plugin_id)?;
        let id = existing.map_or_else(new_id, |id| Ok(id.to_owned()))?;
        self.save_connection(
            &id,
            plugin_id.into(),
            BTreeMap::new(),
            auth,
            Some(PASTED.into()),
        )
    }
    fn ensure_installed(&self, plugin_id: &str) -> Result<()> {
        if self.registry.installed_package(plugin_id)?.is_none() {
            self.install(plugin_id, true)?;
        }
        Ok(())
    }
    fn save_connection(
        &self,
        id: &str,
        plugin_id: String,
        config: BTreeMap<String, String>,
        auth: AuthMaterial,
        source: Option<String>,
    ) -> Result<AuthProgress> {
        let mut runtime = self.registry.load(&plugin_id)?;
        let hosts = runtime.package.manifest.hosts.clone();
        let transport = Transport::new()?;
        let Outcome::Identity(identity) =
            runtime.execute(Operation::Identity, config.clone(), |r| {
                transport.send(r, &hosts, &auth, None, None)
            })?
        else {
            bail!("Provider did not return an account identity")
        };
        // Taken before reading, so two refreshes finding the same account don't both add it.
        let tx = self.vault.write()?;
        let connections = self.vault.list::<Connection>("connection")?;
        let same = |c: &Connection| {
            c.view.plugin_id == plugin_id
                && c.identity_id == identity.id
                && c.view.config.get("team_id") == config.get("team_id")
        };
        let flow_id = id;
        let previous = connections.iter().find(|c| c.view.id == id);
        if let Some(previous) = previous {
            ensure!(
                same(previous),
                "Reconnect the same account and team. To use a different account, connect it from Providers."
            );
        }
        // Connecting an account that is already known renews it instead of adding a duplicate.
        let previous = previous.or_else(|| connections.iter().find(|c| same(c)));
        let id = previous.map_or_else(|| id.to_owned(), |c| c.view.id.clone());
        let mut linked = previous.map(|c| c.linked.clone()).unwrap_or_default();
        for s in previous
            .and_then(|c| c.view.source.clone())
            .into_iter()
            .chain(source.clone())
        {
            if s != PASTED && s != DEDICATED && !linked.contains(&s) {
                linked.push(s);
            }
        }
        let connection = Connection {
            identity_id: identity.id,
            linked,
            dedicated: None,
            view: ConnectionView {
                id: id.clone(),
                plugin_id,
                name: discover::friendly_name(&identity.name),
                state: ConnectionState::Connected,
                config,
                source,
            },
            auth,
        };
        let replaced = previous
            .filter(|p| {
                borrowed(p.view.source.as_deref()) && p.view.source != connection.view.source
            })
            .and_then(|_| {
                self.vault
                    .get::<Credential>("credential", &format!("signin-{id}"))
                    .ok()
            });
        self.vault.put("connection", &id, &connection)?;
        self.save_sign_in(&connection)?;
        if let Some(key) = replaced {
            self.keep_borrowed(&id, key)?;
        }
        self.vault.remove("auth_flow", flow_id)?;
        self.vault.event("account.connected", &id)?;
        tx.commit()?;
        Ok(AuthProgress::Connected {
            connection: connection.view,
        })
    }
    pub fn execute(
        &self,
        id: &str,
        operation: Operation,
        candidate: Option<&SecretValue>,
        injected: Option<&str>,
    ) -> Result<Outcome> {
        let connection: Connection = self.vault.get("connection", id)?;
        let mut runtime = self.registry.load(&connection.view.plugin_id)?;
        let connection = self.renew(id, &runtime.package.manifest)?;
        let transport = Transport::new()?;
        let hosts = runtime.package.manifest.hosts.clone();
        let validating = matches!(operation, Operation::ValidateCredential { .. });
        let outcome = runtime.execute(operation, connection.view.config, |r| {
            transport.send(r, &hosts, &connection.auth, candidate, injected)
        })?;
        if validating && let Outcome::Identity(identity) = &outcome {
            ensure!(
                identity.id == connection.identity_id,
                "This credential belongs to a different provider account"
            );
        }
        Ok(outcome)
    }
    /// Loads a connection with fresh authorization, keeping its sign-in key current.
    fn renew(&self, id: &str, manifest: &Manifest) -> Result<Connection> {
        let current: Connection = self.vault.get("connection", id)?;
        if current.view.state == ConnectionState::Connected && !auth::expiring(&current.auth) {
            return Ok(current);
        }
        // Held across the refresh so parallel calls don't spend the same refresh token twice.
        let tx = self.vault.write()?;
        let mut connection: Connection = self.vault.get("connection", id)?;
        ensure!(
            connection.view.state != ConnectionState::Disconnected,
            "Reconnect this account before using the provider"
        );
        // Sessions borrowed from a local CLI are renewed by re-reading the CLI's current session.
        if auth::expiring(&connection.auth)
            && let Some(source) = connection
                .view
                .source
                .as_deref()
                .filter(|s| ![PASTED, DEDICATED].contains(s))
            && let Ok((_, Found::Material(auth))) = discover::load(&self.manifests(), source)
        {
            connection.auth = *auth;
        }
        if let Err(error) = auth::refresh(&mut connection, manifest) {
            connection.view.state = ConnectionState::NeedsReauth;
            self.vault.put("connection", id, &connection)?;
            tx.commit()?;
            return Err(error);
        }
        connection.view.state = ConnectionState::Connected;
        self.vault.put("connection", id, &connection)?;
        self.save_sign_in(&connection)?;
        tx.commit()?;
        Ok(connection)
    }
    fn save_sign_in(&self, connection: &Connection) -> Result<Credential> {
        let id = format!("signin-{}", connection.view.id);
        let previous = self.vault.get::<Credential>("credential", &id).ok();
        let source = connection.view.source.as_deref();
        let mut details = previous
            .as_ref()
            .map(|c| c.details.clone())
            .unwrap_or_default();
        let (provider_id, fields, expires_at, name) = match &connection.auth {
            AuthMaterial::Bearer {
                token, expires_at, ..
            } => {
                // The provider's ID for the token, once a listing or GetKeys itself named it.
                let provider_id = connection.dedicated.clone().unwrap_or_else(|| {
                    previous
                        .as_ref()
                        .filter(|p| {
                            p.value.as_ref().and_then(|v| v.fields.get("token")) == Some(token)
                        })
                        .map_or_else(|| "sign-in".to_owned(), |p| p.view.provider_id.clone())
                });
                if previous
                    .as_ref()
                    .is_some_and(|p| p.view.provider_id != provider_id)
                {
                    details.clear();
                }
                (
                    provider_id,
                    BTreeMap::from([("token".to_owned(), token.clone())]),
                    *expires_at,
                    discover::sign_in_name(&self.manifests(), source),
                )
            }
            AuthMaterial::Aws {
                access_key_id,
                secret_access_key,
                session_token,
                expires_at,
                sso,
            } => {
                let mut fields = BTreeMap::from([
                    ("access_key_id".to_owned(), access_key_id.clone()),
                    ("secret_access_key".to_owned(), secret_access_key.clone()),
                ]);
                if let Some(token) = session_token {
                    fields.insert("session_token".into(), token.clone());
                }
                details.insert(
                    "account_id".into(),
                    sso.as_ref()
                        .map_or(&connection.identity_id, |s| &s.account_id)
                        .clone(),
                );
                if let Some(region) = discover::aws_region(source) {
                    details.insert("region".into(), region);
                }
                let name = sso.as_ref().map_or_else(
                    || discover::sign_in_name(&self.manifests(), source),
                    |s| format!("{} role", s.role_name),
                );
                (access_key_id.clone(), fields, *expires_at, name)
            }
        };
        let credential = Credential {
            view: CredentialView {
                id: id.clone(),
                connection_id: connection.view.id.clone(),
                provider_id,
                name,
                credential_type: SIGN_IN.into(),
                resource: String::new(),
                project: previous
                    .as_ref()
                    .map(|c| c.view.project.clone())
                    .unwrap_or_default(),
                environment: previous
                    .as_ref()
                    .map(|c| c.view.environment.clone())
                    .unwrap_or_default(),
                created_at: previous.as_ref().and_then(|c| c.view.created_at.clone()),
                expires_at: expires_at.and_then(|e| {
                    time::OffsetDateTime::from_unix_timestamp(e)
                        .ok()?
                        .format(&time::format_description::well_known::Rfc3339)
                        .ok()
                }),
                value_available: true,
                can_rotate: false,
                retired: false,
                fields: Vec::new(),
                resource_name: String::new(),
                groups: previous
                    .as_ref()
                    .map(|c| c.view.groups.clone())
                    .unwrap_or_default(),
                problem: None,
                plugin_id: String::new(),
                origin: sign_in_origin(source).into(),
            },
            value: Some(SecretValue { fields }),
            details,
        };
        let unchanged = previous.is_some_and(|p| {
            serde_json::to_value(&p).ok() == serde_json::to_value(&credential).ok()
        });
        if !unchanged {
            self.vault.put("credential", &id, &credential)?;
        }
        Ok(credential)
    }
    /// Sign-ins and keys on this device that aren't in GetKeys yet and save without a browser.
    /// Adding a provider looks for just its own; a refresh (`None`) looks for every provider
    /// with an active account or installed, and leaves sign-ins the person disconnected alone.
    pub fn scan_sources(&self, plugin: Option<&str>) -> Result<Vec<Discovery>> {
        if let Some(p) = plugin {
            self.ensure_installed(p)?;
        }
        let connections = self.vault.list::<Connection>("connection")?;
        let credentials = self.vault.list::<Credential>("credential")?;
        let active = |c: &Connection| c.view.state != ConnectionState::Disconnected;
        let installed = self.installed_ids();
        let wanted = |d: &Discovery| match plugin {
            Some(p) => d.plugin_id == p,
            None if d.key => installed.contains(&d.plugin_id),
            None => connections
                .iter()
                .any(|c| active(c) && c.view.plugin_id == d.plugin_id),
        };
        let known = |source: &str| {
            connections.iter().any(|c| {
                (plugin.is_none() || active(c))
                    && (c.view.source.as_deref() == Some(source)
                        || c.linked.iter().any(|l| l == source))
            })
        };
        let manifests = self.manifests();
        let fresh = |d: &Discovery, value: &SecretValue| {
            new_key(&credentials, &format!("found:{}", d.id), value)
        };
        // The same key is often in several places; it is offered once.
        let mut seen = Vec::new();
        let found: Vec<Discovery> = discover::scan(&manifests)
            .into_iter()
            .filter(|d| wanted(d) && (d.key || !known(&d.id)))
            .filter(|d| match discover::load(&manifests, &d.id) {
                // Browser sign-ins (IAM Identity Center) need the person, so they stay in Connect.
                Ok((_, Found::Material(_))) => true,
                Ok((_, Found::Key(value))) => {
                    let new = fresh(d, &value) && !seen.contains(&value.fields);
                    seen.push(value.fields);
                    new
                }
                _ => false,
            })
            .collect();
        Ok(found)
    }
    /// Text to put on the clipboard: one field's value, or every exported field as `.env` lines.
    pub fn clip(&self, id: &str, field: &str) -> Result<String> {
        let c: Credential = self.vault.get("credential", id)?;
        let fields = self.fields(&c)?;
        if field != DOTENV
            && let Some(value) = fields
                .iter()
                .find(|f| f.key == field)
                .and_then(|f| f.value.clone())
        {
            return Ok(value);
        }
        // Reading the secret can renew a sign-in key, so its fields are re-read afterwards.
        let secret = self.secret(id)?;
        let c: Credential = self.vault.get("credential", id)?;
        if field != DOTENV {
            return secret
                .fields
                .get(field)
                .cloned()
                .context("This field isn't stored on this device");
        }
        let lines: Vec<String> = self
            .fields(&c)?
            .into_iter()
            .filter(|f| !f.env.is_empty())
            .filter_map(|f| {
                let value = f.value.or_else(|| secret.fields.get(&f.key).cloned())?;
                Some(format!("{}={}", f.env, dotenv_value(&value)))
            })
            .collect();
        ensure!(!lines.is_empty(), "This key has no values to export");
        Ok(lines.join("\n") + "\n")
    }
    fn fields(&self, c: &Credential) -> Result<Vec<KeyField>> {
        if !c.view.plugin_id.is_empty() {
            return Ok(key_fields(&self.manifest(&c.view.plugin_id)?, c));
        }
        if c.view.connection_id.is_empty() {
            return Ok(custom_fields(c));
        }
        let connection: Connection = self.vault.get("connection", &c.view.connection_id)?;
        Ok(key_fields(&self.manifest(&connection.view.plugin_id)?, c))
    }
    /// The chosen fields as a `.env` file.
    pub fn export_env(&self, lines: &[EnvLine]) -> Result<String> {
        ensure!(!lines.is_empty(), "Choose at least one key to export");
        let mut seen = std::collections::BTreeSet::new();
        let mut out = String::new();
        for line in lines {
            ensure!(
                valid_variable(&line.variable),
                "{} isn't a valid variable name",
                line.variable
            );
            ensure!(
                seen.insert(line.variable.as_str()),
                "{} is used by two keys. Rename one of them.",
                line.variable
            );
            let value = self.clip(&line.credential_id, &line.field)?;
            out.push_str(&format!("{}={}\n", line.variable, dotenv_value(&value)));
        }
        self.vault
            .event("keys.exported", &format!("{} values", lines.len()))?;
        Ok(out)
    }
    /// The chosen fields' values by variable name, for a program started with them.
    pub fn resolve(&self, lines: &[EnvLine]) -> Result<BTreeMap<String, String>> {
        let mut values = BTreeMap::new();
        for line in lines {
            ensure!(
                valid_variable(&line.variable) && line.field != DOTENV,
                "{} isn't a valid variable name",
                line.variable
            );
            let value = self.clip(&line.credential_id, &line.field)?;
            ensure!(
                values.insert(line.variable.clone(), value).is_none(),
                "{} is used by two keys. Rename one of them.",
                line.variable
            );
        }
        Ok(values)
    }
    /// Asks the provider whether the key still works. Only a rejection marks it invalid; a
    /// network failure leaves it as it was.
    pub fn check(&self, id: &str) -> Result<Option<String>> {
        let c: Credential = self.vault.get("credential", id)?;
        let Some(value) = c.value.clone() else {
            return Ok(c.view.problem);
        };
        if c.view.retired || c.view.credential_type == SIGN_IN {
            return Ok(c.view.problem);
        }
        if c.view.connection_id.is_empty() {
            if c.view.plugin_id.is_empty() {
                return Ok(c.view.problem);
            }
            let problem = match crate::http::probe(&self.manifest(&c.view.plugin_id)?, &value)? {
                Some(false) => Some(REJECTED.to_owned()),
                Some(true) => None,
                None => return Ok(c.view.problem),
            };
            return self.set_problem(id, problem);
        }
        let result = self.execute(
            &c.view.connection_id,
            Operation::ValidateCredential {
                resource: c.view.resource.clone(),
            },
            Some(&value),
            None,
        );
        let problem = match result {
            Ok(_) => None,
            Err(e) => {
                let cause = e.root_cause().to_string();
                if cause.starts_with("unauthorized:") || cause.starts_with("forbidden:") {
                    Some(REJECTED.to_owned())
                } else if cause.contains("different provider account") {
                    None
                } else {
                    return Err(e);
                }
            }
        };
        self.set_problem(id, problem)
    }
    fn set_problem(&self, id: &str, problem: Option<String>) -> Result<Option<String>> {
        let mut current: Credential = self.vault.get("credential", id)?;
        if current.view.problem != problem {
            current.view.problem = problem.clone();
            self.vault.put("credential", id, &current)?;
        }
        Ok(problem)
    }
    pub fn set_groups(&self, id: &str, groups: Vec<String>) -> Result<CredentialView> {
        let mut c: Credential = self.vault.get("credential", id)?;
        let mut clean: Vec<String> = Vec::new();
        for g in groups {
            let g = g.trim().to_owned();
            ensure!(
                g.chars().count() <= 40,
                "Keep group names under 40 characters"
            );
            if !g.is_empty() && !clean.iter().any(|x| x.eq_ignore_ascii_case(&g)) {
                clean.push(g);
            }
        }
        c.view.groups = clean;
        self.vault.put("credential", id, &c)?;
        Ok(c.view)
    }
    /// Saves a key that belongs to no provider. Editing keeps secret values left blank.
    pub fn save_custom(
        &self,
        id: Option<&str>,
        name: &str,
        fields: Vec<CustomField>,
    ) -> Result<CredentialView> {
        ensure!(!name.trim().is_empty(), "Give the key a name");
        let previous = match id {
            Some(id) => {
                let c: Credential = self.vault.get("credential", id)?;
                ensure!(
                    c.view.credential_type == CUSTOM,
                    "Only keys without a provider can be edited"
                );
                Some(c)
            }
            None => None,
        };
        let kept = |key: &str| {
            previous
                .as_ref()
                .and_then(|c| c.value.as_ref())
                .and_then(|v| v.fields.get(key))
                .cloned()
        };
        let mut secrets = BTreeMap::new();
        let mut details = BTreeMap::new();
        for f in fields {
            let variable = f.variable.trim().to_owned();
            ensure!(
                valid_variable(&variable),
                "Use a variable name such as API_KEY for each value"
            );
            ensure!(
                !secrets.contains_key(&variable) && !details.contains_key(&variable),
                "{variable} is listed twice"
            );
            let value = if f.value.trim().is_empty() && f.secret {
                kept(&variable).context(format!("Enter a value for {variable}"))?
            } else {
                ensure!(!f.value.trim().is_empty(), "Enter a value for {variable}");
                f.value
            };
            if f.secret {
                secrets.insert(variable, value);
            } else {
                details.insert(variable, value);
            }
        }
        ensure!(
            !secrets.is_empty() || !details.is_empty(),
            "Add at least one value"
        );
        let created = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .ok();
        let c = Credential {
            view: CredentialView {
                id: previous
                    .as_ref()
                    .map_or_else(new_id, |c| Ok(c.view.id.clone()))?,
                connection_id: String::new(),
                provider_id: CUSTOM.into(),
                name: name.trim().into(),
                credential_type: CUSTOM.into(),
                resource: String::new(),
                project: String::new(),
                environment: String::new(),
                created_at: previous
                    .as_ref()
                    .map_or(created, |c| c.view.created_at.clone()),
                expires_at: None,
                value_available: true,
                can_rotate: false,
                retired: false,
                fields: Vec::new(),
                resource_name: String::new(),
                groups: previous
                    .as_ref()
                    .map(|c| c.view.groups.clone())
                    .unwrap_or_default(),
                problem: None,
                plugin_id: String::new(),
                origin: origin::PASTED.into(),
            },
            value: Some(SecretValue { fields: secrets }),
            details,
        };
        self.vault.put("credential", &c.view.id, &c)?;
        self.vault.event(
            if previous.is_some() {
                "credential.updated"
            } else {
                "credential.added"
            },
            &c.view.id,
        )?;
        Ok(c.view)
    }
    /// Lists the account's resources and keeps the names its keys show current.
    pub fn resources(&self, id: &str) -> Result<Vec<Resource>> {
        let listing = self.execute(id, Operation::ListResources, None, None);
        if let Err(e) = &listing
            && refused(e)
        {
            self.mark_unchecked(id, None)?;
        }
        let Outcome::Resources(resources) = listing? else {
            bail!("Provider did not return resources")
        };
        for mut c in self.vault.list::<Credential>("credential")? {
            if c.view.connection_id == id
                && let Some(r) = resources.iter().find(|r| r.id == c.view.resource)
                && c.view.resource_name != r.name
            {
                c.view.resource_name = r.name.clone();
                self.vault.put("credential", &c.view.id, &c)?;
            }
        }
        Ok(resources)
    }
    /// Lists a resource's keys. Keys the provider no longer lists were deleted there, so they
    /// are retired here too.
    /// An account the provider won't let list a resource's keys leaves them as they were,
    /// marked as unchecked, rather than failing.
    pub fn sync(&self, id: &str, resource: &str) -> Result<Vec<CredentialView>> {
        let listing = self.execute(
            id,
            Operation::ListCredentials {
                resource: resource.into(),
            },
            None,
            None,
        );
        let listing = match listing {
            Err(e) if refused(&e) => {
                self.mark_unchecked(id, Some(resource))?;
                return self.keys_of(id, resource);
            }
            other => other?,
        };
        let Outcome::Credentials(credentials) = listing else {
            bail!("Provider did not return credentials")
        };
        let listed: Vec<String> = credentials.iter().map(|c| c.id.clone()).collect();
        let rotations = self.vault.rotations()?;
        let tx = self.vault.write()?;
        for c in credentials {
            self.save_remote(id, c)?;
        }
        for c in self.vault.list::<Credential>("credential")? {
            if c.view.connection_id == id
                && c.view.resource == resource
                && !c.view.retired
                && c.view.credential_type != SIGN_IN
                && !c.view.provider_id.starts_with("import-")
                && !listed.contains(&c.view.provider_id)
                && !rotations
                    .iter()
                    .any(|r| r.is_active() && r.uses_credential(&c.view.id))
            {
                crate::rotation::mark_retired(&self.vault, &c)?;
            }
        }
        tx.commit()?;
        self.keys_of(id, resource)
    }
    fn keys_of(&self, id: &str, resource: &str) -> Result<Vec<CredentialView>> {
        Ok(self
            .vault
            .list::<Credential>("credential")?
            .into_iter()
            .filter(|c| c.view.connection_id == id && c.view.resource == resource)
            .map(|c| c.view)
            .collect())
    }
    /// Marks an account's keys, or those of one resource, as ones it isn't allowed to list.
    fn mark_unchecked(&self, id: &str, resource: Option<&str>) -> Result<()> {
        for mut c in self.vault.list::<Credential>("credential")? {
            if c.view.connection_id == id
                && resource.is_none_or(|r| c.view.resource == r)
                && !c.view.retired
                && c.view.credential_type != SIGN_IN
                && c.view.problem.is_none()
            {
                c.view.problem = Some(NO_PERMISSION.into());
                self.vault.put("credential", &c.view.id, &c)?;
            }
        }
        Ok(())
    }
    pub(crate) fn save_remote(
        &self,
        connection_id: &str,
        remote: RemoteCredential,
    ) -> Result<Credential> {
        crate::rotation::save_remote(&self.vault, connection_id, remote)
    }
    /// A key borrowed from this device stays usable there, so once its account signs in with
    /// another key it is kept as an ordinary key.
    fn keep_borrowed(&self, connection_id: &str, key: Credential) -> Result<()> {
        let aws = key
            .value
            .as_ref()
            .is_some_and(|v| v.fields.contains_key("access_key_id"));
        self.save_remote(
            connection_id,
            RemoteCredential {
                id: if key.view.provider_id == "sign-in" {
                    format!("import-{}", new_id()?)
                } else {
                    key.view.provider_id
                },
                name: key.view.name,
                credential_type: if aws {
                    "iam_access_key"
                } else {
                    "access_token"
                }
                .into(),
                resource: String::new(),
                created_at: key.view.created_at,
                expires_at: key.view.expires_at,
                value: key.value,
                can_rotate: false,
                details: key.details,
            },
        )?;
        Ok(())
    }
    pub fn import(&self, input: ImportInput) -> Result<CredentialView> {
        ensure!(!input.name.trim().is_empty(), "Credential name is required");
        let fresh = input.provider_id.is_none();
        if let Some(existing) = self
            .vault
            .list::<Credential>("credential")?
            .iter()
            .find(|c| {
                c.view.connection_id == input.connection_id
                    && Some(&c.view.provider_id) == input.provider_id.as_ref()
            })
        {
            ensure!(
                !self
                    .vault
                    .rotations()?
                    .iter()
                    .any(|r| r.is_active() && r.uses_credential(&existing.view.id)),
                "Finish the active rotation before replacing this saved value"
            );
        }
        let value = SecretValue {
            fields: input.fields,
        };
        if let (Some(provider_id), Some(access_key_id)) =
            (&input.provider_id, value.fields.get("access_key_id"))
        {
            ensure!(
                provider_id == access_key_id,
                "The imported access key ID does not match the selected credential"
            );
        }
        ensure!(
            !value.fields.is_empty() && value.fields.values().all(|s| !s.trim().is_empty()),
            "Credential fields cannot be empty"
        );
        self.execute(
            &input.connection_id,
            Operation::ValidateCredential {
                resource: input.resource.clone(),
            },
            Some(&value),
            None,
        )?;
        let remote = RemoteCredential {
            id: input.provider_id.unwrap_or(format!("import-{}", new_id()?)),
            name: input.name,
            credential_type: "imported".into(),
            resource: input.resource,
            created_at: None,
            expires_at: None,
            value: Some(value),
            can_rotate: false,
            details: BTreeMap::new(),
        };
        let mut c = self.save_remote(&input.connection_id, remote)?;
        c.view.project = input.project;
        c.view.environment = input.environment;
        if fresh {
            c.view.origin = origin::PASTED.into();
        }
        self.vault.put("credential", &c.view.id, &c)?;
        self.vault.event("credential.imported", &c.view.id)?;
        Ok(c.view)
    }
    pub fn create(&self, connection: &str, resource: &str, name: &str) -> Result<CredentialView> {
        ensure!(!name.trim().is_empty(), "Credential name is required");
        let Outcome::Credential(remote) = self.execute(
            connection,
            Operation::CreateCredential {
                resource: resource.into(),
                name: name.into(),
            },
            None,
            None,
        )?
        else {
            bail!("Provider did not return a new credential")
        };
        ensure!(
            remote.value.is_some(),
            "Provider did not return the new credential value. Inspect the provider before trying again."
        );
        let mut c = self.save_remote(connection, remote)?;
        c.view.origin = origin::CREATED.into();
        self.vault.put("credential", &c.view.id, &c)?;
        self.vault.event("credential.created", &c.view.id)?;
        Ok(c.view)
    }
    pub fn secret(&self, id: &str) -> Result<SecretValue> {
        let mut c: Credential = self.vault.get("credential", id)?;
        ensure!(!c.view.retired, "This credential is retired");
        if c.view.credential_type == SIGN_IN {
            let connection: Connection = self.vault.get("connection", &c.view.connection_id)?;
            self.renew(
                &c.view.connection_id,
                &self.manifest(&connection.view.plugin_id)?,
            )?;
            c = self.vault.get("credential", id)?;
        }
        if c.value.is_none() {
            let Outcome::Credential(remote) = self.execute(
                &c.view.connection_id,
                Operation::ReadCredential {
                    resource: c.view.resource.clone(),
                    credential: c.view.provider_id.clone(),
                },
                None,
                None,
            )?
            else {
                bail!("Provider did not return a credential")
            };
            c = self.save_remote(&c.view.connection_id, remote)?;
        }
        let value = c.value.context(
            "This provider reveals values only at creation. Import the value to use it here.",
        )?;
        self.vault.event("credential.retrieved", id)?;
        Ok(value)
    }
    pub fn guide(&self, connection: &str, resource: &str) -> Result<String> {
        let Outcome::Guide { url, message } = self.execute(
            connection,
            Operation::Guide {
                resource: resource.into(),
            },
            None,
            None,
        )?
        else {
            bail!("Provider did not return setup instructions")
        };
        let c: Connection = self.vault.get("connection", connection)?;
        crate::http::validate_url(&url, &self.manifest(&c.view.plugin_id)?.hosts)?;
        open::that_detached(url)?;
        Ok(message)
    }
    /// Signs the account in with an access token GetKeys creates for itself, so it no longer
    /// borrows a CLI session or a personal token. Doing it again rotates that token.
    pub fn dedicate(&self, id: &str) -> Result<ConnectionView> {
        let connection: Connection = self.vault.get("connection", id)?;
        let manifest = self.manifest(&connection.view.plugin_id)?;
        ensure!(
            manifest.account_credentials
                && [
                    OperationKind::CreateCredential,
                    OperationKind::RevokeCredential
                ]
                .iter()
                .all(|o| manifest.operations.contains(o)),
            "{} can't create access tokens through its API. Create one on its page and paste it.",
            manifest.name
        );
        ensure!(
            !self
                .vault
                .rotations()?
                .iter()
                .any(|r| r.is_active() && r.uses_connection(id)),
            "Finish active rotations before replacing this account's key"
        );
        let Outcome::Credential(remote) = self.execute(
            id,
            Operation::CreateCredential {
                resource: String::new(),
                name: "GetKeys".into(),
            },
            None,
            None,
        )?
        else {
            bail!("Provider did not return the new token")
        };
        let revoke = |credential: String| {
            self.execute(
                id,
                Operation::RevokeCredential {
                    resource: String::new(),
                    credential,
                },
                None,
                None,
            )
        };
        let checked = remote
            .value
            .as_ref()
            .context("Provider did not return the new token's value")
            .and_then(|value| {
                self.execute(
                    id,
                    Operation::ValidateCredential {
                        resource: String::new(),
                    },
                    Some(value),
                    None,
                )?;
                auth_from_secret(value)
            });
        let auth = match checked {
            Ok(auth) => auth,
            Err(error) => {
                let _ = revoke(remote.id);
                return Err(error);
            }
        };
        let mut connection: Connection = self.vault.get("connection", id)?;
        let previous: Credential = self.vault.get("credential", &format!("signin-{id}"))?;
        let tx = self.vault.write()?;
        let old = connection.dedicated.replace(remote.id);
        let was_borrowed = borrowed(connection.view.source.as_deref());
        if let Some(source) = connection.view.source.take()
            && was_borrowed
            && !connection.linked.contains(&source)
        {
            connection.linked.push(source);
        }
        connection.view.source = Some(DEDICATED.into());
        connection.view.state = ConnectionState::Connected;
        connection.auth = auth;
        self.vault.put("connection", id, &connection)?;
        self.save_sign_in(&connection)?;
        if old.is_none() && (was_borrowed || previous.view.provider_id != "sign-in") {
            self.keep_borrowed(id, previous)?;
        }
        self.vault.event("account.dedicated", id)?;
        tx.commit()?;
        if let Some(old) = old {
            let _ = revoke(old);
        }
        Ok(connection.view)
    }
    /// Deletes a key. The provider revokes it when its API can; otherwise its page opens so the
    /// person can, and [`App::confirm_deleted`] finishes once they have.
    pub fn delete(&self, id: &str) -> Result<Deletion> {
        let c: Credential = self.vault.get("credential", id)?;
        ensure!(
            c.view.credential_type != SIGN_IN,
            "Remove the account in Providers to delete the key it signs in with"
        );
        ensure!(
            !self
                .vault
                .rotations()?
                .iter()
                .any(|r| r.is_active() && r.uses_credential(id)),
            "Finish the active rotation before deleting this key"
        );
        let by_value = self.revoke_by_value(&c)?;
        if let Some((connection, value)) = &by_value {
            if !c.view.retired {
                self.execute(
                    connection,
                    Operation::RevokeCredential {
                        resource: c.view.resource.clone(),
                        credential: c.view.provider_id.clone(),
                    },
                    None,
                    Some(value),
                )?;
            }
            self.forget(&c)?;
            return Ok(Deletion::Deleted);
        }
        if c.view.connection_id.is_empty() {
            // A provider's key held on its own is revoked on the provider's page.
            if let Ok(m) = self.manifest(&c.view.plugin_id)
                && !c.view.retired
                && m.token_page.starts_with("https://")
            {
                open::that_detached(&m.token_page).context("Unable to open the system browser")?;
                return Ok(Deletion::Manual { url: m.token_page });
            }
            self.forget(&c)?;
            return Ok(Deletion::Deleted);
        }
        let connection: Connection = self.vault.get("connection", &c.view.connection_id)?;
        let manifest = self.manifest(&connection.view.plugin_id)?;
        if !c.view.retired {
            if !c.view.provider_id.starts_with("import-")
                && !manifest.revoke_by_value
                && manifest
                    .operations
                    .contains(&OperationKind::RevokeCredential)
            {
                self.execute(
                    &c.view.connection_id,
                    Operation::RevokeCredential {
                        resource: c.view.resource.clone(),
                        credential: c.view.provider_id.clone(),
                    },
                    None,
                    None,
                )?;
            } else if manifest.token_page.starts_with("https://") {
                open::that_detached(&manifest.token_page)
                    .context("Unable to open the system browser")?;
                return Ok(Deletion::Manual {
                    url: manifest.token_page,
                });
            }
        }
        self.forget(&c)?;
        Ok(Deletion::Deleted)
    }
    /// The account to revoke a key through and its value, for providers that revoke keys by
    /// value. Any of the provider's connected accounts can revoke a key held on its own.
    fn revoke_by_value(&self, c: &Credential) -> Result<Option<(String, String)>> {
        let plugin = if c.view.connection_id.is_empty() {
            c.view.plugin_id.clone()
        } else {
            self.vault
                .get::<Connection>("connection", &c.view.connection_id)?
                .view
                .plugin_id
        };
        let Ok(manifest) = self.manifest(&plugin) else {
            return Ok(None);
        };
        if !manifest.revoke_by_value
            || !manifest
                .operations
                .contains(&OperationKind::RevokeCredential)
        {
            return Ok(None);
        }
        let Some(value) = manifest
            .credential_fields
            .iter()
            .filter(|f| f.secret)
            .find_map(|f| c.value.as_ref()?.fields.get(&f.key))
        else {
            return Ok(None);
        };
        let connection = if c.view.connection_id.is_empty() {
            self.vault
                .list::<Connection>("connection")?
                .into_iter()
                .find(|x| x.view.plugin_id == plugin && x.view.state == ConnectionState::Connected)
                .map(|x| x.view.id)
        } else {
            Some(c.view.connection_id.clone())
        };
        Ok(connection.map(|id| (id, value.clone())))
    }
    /// Finishes deleting a key the person revoked on the provider's page.
    pub fn confirm_deleted(&self, id: &str) -> Result<()> {
        let c: Credential = self.vault.get("credential", id)?;
        if c.view.connection_id.is_empty() {
            return self.forget(&c);
        }
        let connection: Connection = self.vault.get("connection", &c.view.connection_id)?;
        let manifest = self.manifest(&connection.view.plugin_id)?;
        if !c.view.provider_id.starts_with("import-")
            && manifest
                .operations
                .contains(&OperationKind::ListCredentials)
        {
            let listed = self.sync(&c.view.connection_id, &c.view.resource)?;
            ensure!(
                listed.iter().all(|k| k.id != id || k.retired),
                "{} still lists this key. Delete it there, then try again.",
                manifest.name
            );
        }
        self.forget(&c)
    }
    fn forget(&self, c: &Credential) -> Result<()> {
        let tx = self.vault.write()?;
        crate::rotation::mark_retired(&self.vault, c)?;
        self.vault.event("credential.deleted", &c.view.id)?;
        tx.commit()?;
        Ok(())
    }
    pub fn disconnect(&self, id: &str) -> Result<()> {
        ensure!(
            !self
                .vault
                .rotations()?
                .iter()
                .any(|r| r.is_active() && r.uses_connection(id)),
            "Finish active rotations before disconnecting"
        );
        // The token GetKeys created for itself is no longer needed by anyone.
        if let Some(token) = self.vault.get::<Connection>("connection", id)?.dedicated {
            let _ = self.execute(
                id,
                Operation::RevokeCredential {
                    resource: String::new(),
                    credential: token,
                },
                None,
                None,
            );
        }
        let mut connection: Connection = self.vault.get("connection", id)?;
        connection.dedicated = None;
        connection.view.state = ConnectionState::Disconnected;
        connection.auth = AuthMaterial::Bearer {
            token: String::new(),
            refresh_token: None,
            expires_at: None,
            broker_registration: None,
        };
        self.vault.put("connection", id, &connection)?;
        // The key it signed in with leaves the device with it.
        if let Ok(mut key) = self
            .vault
            .get::<Credential>("credential", &format!("signin-{id}"))
        {
            key.value = None;
            key.view.value_available = false;
            key.view.retired = true;
            self.vault.put("credential", &key.view.id, &key)?;
        }
        self.vault.event("account.disconnected", id)
    }
}

/// Field name for [`App::clip`] that copies every exported field as `.env` lines.
pub const DOTENV: &str = ".env";

/// `credential_type` of a key that belongs to no provider.
pub const CUSTOM: &str = "custom";

/// `credential_type` of a provider's key held on its own, outside any account.
pub const API_KEY: &str = "api_key";

const REJECTED: &str = "Rejected by the provider. It was revoked or is wrong.";

/// Problem of a sign-in found on this device that works but can't read its own account.
const CANT_SIGN_IN: &str = "It can't read its own account, so GetKeys can't sign in with it";

/// Why GetKeys can't replace a key itself: the provider makes keys only on its own page.
pub const MAKE_ON_PAGE: &str = "This provider makes keys only on its own page";

/// Problem of a key the account isn't allowed to list, so it can't be checked.
pub const NO_PERMISSION: &str = "This account isn't allowed to list it, so it can't be checked";

/// Whether a key found on this device is new or changed. Keys the person deleted stay deleted,
/// and a value GetKeys already holds elsewhere isn't listed twice.
fn new_key(credentials: &[Credential], id: &str, value: &SecretValue) -> bool {
    if let Some(saved) = credentials.iter().find(|c| c.view.id == id) {
        return !saved.view.retired
            && saved.value.as_ref().map(|v| &v.fields) != Some(&value.fields);
    }
    let Some(primary) = value.fields.values().next() else {
        return false;
    };
    !credentials.iter().any(|c| {
        !c.view.retired
            && c.value
                .as_ref()
                .is_some_and(|v| v.fields.values().any(|x| x == primary))
    })
}

/// Why a key no longer works, from what is known without asking the provider.
fn problem(
    c: &Credential,
    connection: Option<&Connection>,
    now: time::OffsetDateTime,
) -> Option<String> {
    if c.view.credential_type == SIGN_IN {
        return connection
            .filter(|x| x.view.state == ConnectionState::NeedsReauth)
            .map(|_| "The provider no longer accepts it. Sign in again.".to_owned());
    }
    let expired = c.view.expires_at.as_deref().is_some_and(|e| {
        time::OffsetDateTime::parse(e, &time::format_description::well_known::Rfc3339)
            .is_ok_and(|e| e <= now)
    });
    if expired {
        return Some("Expired".into());
    }
    if c.details
        .get("status")
        .is_some_and(|s| s.eq_ignore_ascii_case("inactive"))
    {
        return Some("Deactivated at the provider".into());
    }
    c.view.problem.clone()
}

/// Whether the provider refused the account, as opposed to failing or being unreachable.
pub fn refused(e: &anyhow::Error) -> bool {
    let cause = e.root_cause().to_string();
    cause.starts_with("forbidden:") || cause.contains(": forbidden:")
}

/// Whether the provider turned the key away as revoked, expired, or wrong.
fn unauthorized(e: &anyhow::Error) -> bool {
    let cause = e.root_cause().to_string();
    cause.starts_with("unauthorized:") || cause.contains(": unauthorized:")
}

/// The values of a sign-in, as the fields of a key.
fn material_fields(auth: &AuthMaterial) -> BTreeMap<String, String> {
    match auth {
        AuthMaterial::Bearer { token, .. } => BTreeMap::from([("token".to_owned(), token.clone())]),
        AuthMaterial::Aws {
            access_key_id,
            secret_access_key,
            session_token,
            ..
        } => {
            let mut fields = BTreeMap::from([
                ("access_key_id".to_owned(), access_key_id.clone()),
                ("secret_access_key".to_owned(), secret_access_key.clone()),
            ]);
            if let Some(token) = session_token {
                fields.insert("session_token".into(), token.clone());
            }
            fields
        }
    }
}

/// How an account's sign-in key got here, from the account's source.
fn sign_in_origin(source: Option<&str>) -> &'static str {
    match source {
        None => origin::SIGNED_IN,
        Some(DEDICATED) => origin::CREATED,
        s if borrowed(s) => origin::FOUND,
        _ => origin::PASTED,
    }
}

/// How a key saved by an earlier version, which didn't record it, most likely got here.
fn legacy_origin(c: &Credential, connection: Option<&Connection>) -> &'static str {
    if c.view.id.starts_with("found:") {
        origin::FOUND
    } else if c.view.credential_type == SIGN_IN {
        sign_in_origin(connection.and_then(|x| x.view.source.as_deref()))
    } else if c.view.credential_type == CUSTOM
        || !c.view.plugin_id.is_empty()
        || c.view.provider_id.starts_with("import-")
    {
        origin::PASTED
    } else {
        origin::LISTED
    }
}

/// A key without a provider: every value is exported under its own name.
fn custom_fields(c: &Credential) -> Vec<KeyField> {
    let plain = c.details.iter().map(|(k, v)| KeyField {
        key: k.clone(),
        label: k.clone(),
        env: k.clone(),
        secret: false,
        value: Some(v.clone()),
        available: true,
    });
    let secret = c
        .value
        .iter()
        .flat_map(|v| v.fields.keys())
        .map(|k| KeyField {
            key: k.clone(),
            label: k.clone(),
            env: k.clone(),
            secret: true,
            value: None,
            available: true,
        });
    secret.chain(plain).collect()
}

/// Whether an account signs in with a key it found on this device (a CLI, profile, or variable).
fn borrowed(source: Option<&str>) -> bool {
    source.is_some_and(|s| ![PASTED, DEDICATED].contains(&s))
}

/// Lays a key out by its plugin's field schema: declared fields first, then any extras.
pub fn key_fields(manifest: &Manifest, c: &Credential) -> Vec<KeyField> {
    let schema = if c.view.credential_type == SIGN_IN {
        &manifest.account_fields
    } else {
        &manifest.credential_fields
    };
    let stored = |key: &str| c.value.as_ref().and_then(|v| v.fields.get(key));
    let mut fields: Vec<KeyField> = schema
        .iter()
        .filter_map(|f| {
            let value = c.details.get(&f.key).or_else(|| stored(&f.key));
            (value.is_some() || (f.required && f.secret)).then(|| KeyField {
                key: f.key.clone(),
                label: f.label.clone(),
                env: f.env.clone(),
                secret: f.secret,
                value: value.filter(|_| !f.secret).cloned(),
                available: value.is_some(),
            })
        })
        .collect();
    let known = |key: &str| schema.iter().any(|f| f.key == key);
    for key in c.value.iter().flat_map(|v| v.fields.keys()) {
        if !known(key) {
            fields.push(KeyField {
                key: key.clone(),
                label: label(key),
                env: String::new(),
                secret: true,
                value: None,
                available: true,
            });
        }
    }
    for (key, value) in &c.details {
        if !known(key) && !fields.iter().any(|f| f.key == *key) {
            fields.push(KeyField {
                key: key.clone(),
                label: label(key),
                env: String::new(),
                secret: false,
                value: Some(value.clone()),
                available: true,
            });
        }
    }
    fields
}

fn label(key: &str) -> String {
    let words: Vec<String> = key
        .split('_')
        .enumerate()
        .map(|(i, w)| match w {
            "id" | "url" | "arn" => w.to_uppercase(),
            _ if i == 0 => {
                let mut chars = w.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            }
            _ => w.to_owned(),
        })
        .collect();
    words.join(" ")
}

fn dotenv_value(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./:+=@,".contains(c))
    {
        value.to_owned()
    } else {
        format!(
            "\"{}\"",
            value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        )
    }
}

pub fn valid_variable(s: &str) -> bool {
    let mut bytes = s.bytes();
    bytes
        .next()
        .is_some_and(|b| b == b'_' || b.is_ascii_alphabetic())
        && bytes.all(|b| b == b'_' || b.is_ascii_alphanumeric())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let app = App {
            vault: Vault::open(&dir.path().join("vault.db"), &[3; 32]).unwrap(),
            registry: Registry::open(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../registry/dist"),
                dir.path().join("plugins"),
            )
            .unwrap(),
        };
        (dir, app)
    }
    fn field(variable: &str, value: &str, secret: bool) -> CustomField {
        CustomField {
            variable: variable.into(),
            value: value.into(),
            secret,
        }
    }
    #[test]
    fn keys_without_a_provider_export_grouped_and_edit_in_place() {
        let (_dir, app) = app();
        let key = app
            .save_custom(
                None,
                "OpenAI",
                vec![
                    field("OPENAI_API_KEY", "sk-test value", true),
                    field("OPENAI_ORG", "org-1", false),
                ],
            )
            .unwrap();
        let fields = &app
            .snapshot()
            .unwrap()
            .credentials
            .into_iter()
            .find(|c| c.id == key.id)
            .unwrap()
            .fields;
        assert_eq!(fields.len(), 2);
        assert!(fields[0].secret && fields[0].value.is_none());
        assert_eq!(fields[1].value.as_deref(), Some("org-1"));
        // A blank secret keeps the stored value.
        app.save_custom(
            Some(&key.id),
            "OpenAI",
            vec![field("OPENAI_API_KEY", "", true)],
        )
        .unwrap();
        assert_eq!(
            app.clip(&key.id, "OPENAI_API_KEY").unwrap(),
            "sk-test value"
        );
        let grouped = app
            .set_groups(&key.id, vec![" web ".into(), "WEB".into(), String::new()])
            .unwrap();
        assert_eq!(grouped.groups, ["web"]);
        let line = |variable: &str| EnvLine {
            credential_id: key.id.clone(),
            field: "OPENAI_API_KEY".into(),
            variable: variable.into(),
        };
        assert_eq!(
            app.export_env(&[line("KEY")]).unwrap(),
            "KEY=\"sk-test value\"\n"
        );
        assert!(app.export_env(&[line("KEY"), line("KEY")]).is_err());
        assert!(app.export_env(&[line("A B")]).is_err());
        // A second connection sees the same vault.
        let fork = app.fork().unwrap();
        assert!(matches!(fork.delete(&key.id).unwrap(), Deletion::Deleted));
        assert!(app.secret(&key.id).is_err());
    }
    #[test]
    fn key_pages_are_filled_in_and_drop_what_is_unknown() {
        let (_dir, app) = app();
        let url = app
            .key_page("official.github", None, "octo/app", "My key")
            .unwrap();
        assert!(url.starts_with("https://github.com/settings/personal-access-tokens/new?"));
        assert!(url.contains("name=My+key") && url.contains("target_name=octo"));
        let url = app.key_page("official.github", None, "", "web").unwrap();
        assert!(!url.contains("target_name"));
        assert_eq!(
            app.key_page("official.groq", None, "", "x").unwrap(),
            "https://console.groq.com/keys"
        );
    }
    #[test]
    fn captured_keys_join_the_groups_of_the_key_they_replace() {
        let (_dir, app) = app();
        let old = app
            .save_custom(None, "Old", vec![field("KEY", "v", true)])
            .unwrap();
        app.set_groups(&old.id, vec!["web".into()]).unwrap();
        let key = app
            .save_captured(
                "official.supabase",
                None,
                "",
                "New",
                " sb_secret_value ",
                Some("api"),
                Some(&old.id),
            )
            .unwrap();
        assert_eq!(key.origin, origin::CAPTURED);
        assert_eq!(key.groups, ["web", "api"]);
        assert!(
            app.secret(&key.id)
                .unwrap()
                .fields
                .values()
                .any(|v| v == "sb_secret_value")
        );
        // Keys GetKeys can't replace through the provider say so.
        assert!(
            app.rotate_now(&old.id)
                .unwrap_err()
                .to_string()
                .contains(MAKE_ON_PAGE)
        );
    }
    #[test]
    fn environment_names_cannot_inject_shell_syntax() {
        assert!(valid_variable("AWS_SECRET_ACCESS_KEY"));
        for s in ["", "2KEY", "KEY=VALUE", "A;B", "A\nB"] {
            assert!(!valid_variable(s));
        }
    }
    #[test]
    fn keys_follow_the_plugin_schema_without_exposing_secrets() {
        let manifest: Manifest =
            serde_json::from_str(include_str!("../../../plugins/aws/manifest.json")).unwrap();
        let mut c = Credential {
            view: serde_json::from_value(serde_json::json!({
                "id": "c", "connection_id": "a", "provider_id": "AKIA1", "name": "k",
                "credential_type": SIGN_IN, "resource": "", "project": "", "environment": "",
                "created_at": null, "expires_at": null, "value_available": true,
                "can_rotate": false, "retired": false
            }))
            .unwrap(),
            value: Some(SecretValue {
                fields: BTreeMap::from([
                    ("access_key_id".into(), "AKIA1".into()),
                    ("secret_access_key".into(), "shh".into()),
                ]),
            }),
            details: BTreeMap::from([
                ("region".into(), "eu-west-2".into()),
                ("account_id".into(), "123".into()),
                ("status".into(), "Active".into()),
            ]),
        };
        let fields = key_fields(&manifest, &c);
        let keys: Vec<_> = fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "access_key_id",
                "secret_access_key",
                "region",
                "account_id",
                "status"
            ]
        );
        assert_eq!(fields[0].value.as_deref(), Some("AKIA1"));
        assert_eq!(fields[0].env, "AWS_ACCESS_KEY_ID");
        assert!(fields[1].secret && fields[1].value.is_none() && fields[1].available);
        assert_eq!(fields[4].label, "Status");
        // A listed key whose secret is not stored still shows the secret, as unavailable.
        c.view.credential_type = "iam_access_key".into();
        c.value = None;
        c.details = BTreeMap::from([("access_key_id".into(), "AKIA1".into())]);
        let fields = key_fields(&manifest, &c);
        assert_eq!(fields.len(), 2);
        assert!(!fields[1].available);
        assert!(
            serde_json::from_str::<ConfigField>(r#"{"key":"k","label":"K"}"#)
                .unwrap()
                .secret
        );
    }
    #[test]
    fn dotenv_values_are_quoted_only_when_needed() {
        assert_eq!(dotenv_value("AKIA/abc+def="), "AKIA/abc+def=");
        assert_eq!(dotenv_value("a b\"c"), "\"a b\\\"c\"");
        assert_eq!(label("account_id"), "Account ID");
        assert_eq!(label("project_url"), "Project URL");
    }
    #[test]
    fn provider_keys_are_found_rotated_by_hand_and_removed_with_their_provider() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        use ed25519_dalek::{Signer, SigningKey};
        use sha2::Digest;
        const VAR: &str = "GETKEYS_TEST_ACME_KEY";
        let dir = tempfile::tempdir().unwrap();
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "id": "test.acme", "name": "Acme", "publisher": "t", "version": "1", "description": "",
            "homepage": "https://acme.dev", "kind": "declarative", "hosts": ["acme.dev"],
            "token_page": "https://acme.dev/keys",
            "credential_fields": [{"key": "token", "label": "API key", "required": true}],
            "sources": [{"kind": "env", "var": VAR, "role": "key"}]
        }))
        .unwrap();
        let sha256 = hex::encode(sha2::Sha256::digest(crate::plugins::manifest_bytes(
            &manifest,
        )));
        let index = serde_json::to_vec(&crate::plugins::Catalog {
            packages: vec![crate::plugins::Package {
                manifest,
                wasm: String::new(),
                sha256,
            }],
        })
        .unwrap();
        let signing = SigningKey::from_bytes(&[7; 32]);
        let source = dir.path().join("registry");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("index.json"), &index).unwrap();
        std::fs::write(
            source.join("index.sig"),
            STANDARD.encode(signing.sign(&index).to_bytes()),
        )
        .unwrap();
        let app = App {
            vault: Vault::open(&dir.path().join("vault.db"), &[4; 32]).unwrap(),
            registry: Registry::with_key(
                source,
                dir.path().join("plugins"),
                &hex::encode(signing.verifying_key().as_bytes()),
            )
            .unwrap(),
        };
        // SAFETY: no other test reads this variable.
        unsafe { std::env::set_var(VAR, "acme_first") };
        app.install("test.acme", true).unwrap();
        let found = app.scan_sources(Some("test.acme")).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].key);
        let AuthProgress::KeySaved { credential } = app.connect_local(&found[0].id, None).unwrap()
        else {
            panic!("expected a saved key");
        };
        assert_eq!(credential.plugin_id, "test.acme");
        assert!(credential.connection_id.is_empty());
        assert_eq!(app.clip(&credential.id, "token").unwrap(), "acme_first");
        // The same value isn't offered twice.
        assert!(app.scan_sources(Some("test.acme")).unwrap().is_empty());
        // A pasted replacement is kept apart and the place it was found stops bringing it back.
        let rotated = app
            .save_key(
                Some(&credential.id),
                "test.acme",
                "",
                BTreeMap::from([("token".into(), "acme_second".into())]),
            )
            .unwrap();
        assert_ne!(rotated.id, credential.id);
        assert_eq!(app.clip(&rotated.id, "token").unwrap(), "acme_second");
        assert!(app.scan_sources(Some("test.acme")).unwrap().is_empty());
        let manual = app
            .save_key(
                None,
                "test.acme",
                "Laptop",
                BTreeMap::from([("token".into(), "acme_third".into())]),
            )
            .unwrap();
        assert!(
            app.save_key(None, "test.acme", "Empty", BTreeMap::new())
                .is_err()
        );
        let listed = |app: &App| {
            app.snapshot()
                .unwrap()
                .credentials
                .into_iter()
                .filter(|c| !c.retired && c.plugin_id == "test.acme")
                .map(|c| c.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(listed(&app).len(), 2);
        assert!(listed(&app).contains(&manual.id));
        app.uninstall("test.acme").unwrap();
        assert!(listed(&app).is_empty());
        unsafe { std::env::remove_var(VAR) };
    }
}
