use crate::{
    api::*,
    auth::{self, PollResult},
    http::Transport,
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
        Ok(Self {
            vault: Vault::system(&data.join("vault.db"))?,
            registry: Registry::open(registry, data.join("plugins"))?,
        })
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
                    manifest: p.manifest.clone(),
                    installed: installed.is_some(),
                    update_available: installed.is_some_and(|i| i.sha256 != p.sha256),
                })
            })
            .collect::<Result<_>>()?;
        let mut activity = self.vault.list::<Activity>("activity")?;
        activity.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
        activity.truncate(100);
        let mut setup = Vec::new();
        if std::env::var("GETKEYS_GITHUB_CLIENT_ID")
            .ok()
            .is_none_or(|v| v.trim().is_empty())
        {
            setup.push("Set GETKEYS_GITHUB_CLIENT_ID to enable GitHub browser sign-in.".into());
        }
        if std::env::var("GETKEYS_BROKER_URL")
            .ok()
            .is_none_or(|v| v.trim().is_empty())
        {
            setup.push("Set GETKEYS_BROKER_URL to enable Vercel and Supabase browser sign-in. See docs/setup.md.".into());
        }
        Ok(Snapshot {
            plugins,
            connections: self
                .vault
                .list::<Connection>("connection")?
                .into_iter()
                .map(|c| c.view)
                .collect(),
            credentials: self
                .vault
                .list::<Credential>("credential")?
                .into_iter()
                .map(|c| c.view)
                .collect(),
            bindings: self.vault.list("binding")?,
            rotations: self.vault.rotations()?,
            activity,
            setup,
        })
    }
    pub fn manifest(&self, id: &str) -> Result<Manifest> {
        Ok(self
            .registry
            .installed_package(id)?
            .context("Install this plugin first")?
            .manifest)
    }
    pub fn install(&self, id: &str, accept_permissions: bool) -> Result<()> {
        self.ensure_plugin_idle(id)?;
        self.registry.install(id, accept_permissions)?;
        self.vault.event("plugin.installed", id)
    }
    pub fn uninstall(&self, id: &str) -> Result<()> {
        self.ensure_plugin_idle(id)?;
        ensure!(
            !self
                .vault
                .list::<Connection>("connection")?
                .iter()
                .any(|c| c.view.plugin_id == id && c.view.state != ConnectionState::Disconnected),
            "Disconnect this plugin's accounts before removing it"
        );
        self.registry.uninstall(id)?;
        self.vault.event("plugin.removed", id)
    }
    fn ensure_plugin_idle(&self, id: &str) -> Result<()> {
        let connections = self.vault.list::<Connection>("connection")?;
        ensure!(
            !self.vault.rotations()?.iter().any(|r| r.is_active()
                && connections
                    .iter()
                    .any(|c| r.uses_connection(&c.view.id) && c.view.plugin_id == id)),
            "Finish active rotations before changing this plugin"
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
        if let Some(previous) = self
            .vault
            .list::<Connection>("connection")?
            .into_iter()
            .find(|c| c.view.id == id)
        {
            ensure!(
                previous.identity_id == identity.id
                    && previous.view.config.get("team_id") == config.get("team_id"),
                "Reconnect the same account and team. To use a different account, add a new connection from Plugins."
            );
        }
        let connection = Connection {
            identity_id: identity.id,
            view: ConnectionView {
                id: id.into(),
                plugin_id,
                name: identity.name,
                state: ConnectionState::Connected,
                config,
            },
            auth,
        };
        let tx = self.vault.db.unchecked_transaction()?;
        self.vault.put("connection", id, &connection)?;
        self.vault.remove("auth_flow", id)?;
        self.vault.event("account.connected", id)?;
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
        let mut connection: Connection = self.vault.get("connection", id)?;
        ensure!(
            connection.view.state != ConnectionState::Disconnected,
            "Reconnect this account before using the provider"
        );
        let mut runtime = self.registry.load(&connection.view.plugin_id)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.vault.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        connection = self.vault.get("connection", id)?;
        if let Err(error) = auth::refresh(&mut connection, &runtime.package.manifest) {
            connection.view.state = ConnectionState::NeedsReauth;
            self.vault.put("connection", id, &connection)?;
            tx.commit()?;
            return Err(error);
        }
        connection.view.state = ConnectionState::Connected;
        self.vault.put("connection", id, &connection)?;
        tx.commit()?;
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
    pub fn resources(&self, id: &str) -> Result<Vec<Resource>> {
        let Outcome::Resources(resources) =
            self.execute(id, Operation::ListResources, None, None)?
        else {
            bail!("Provider did not return resources")
        };
        Ok(resources)
    }
    pub fn sync(&self, id: &str, resource: &str) -> Result<Vec<CredentialView>> {
        let Outcome::Credentials(credentials) = self.execute(
            id,
            Operation::ListCredentials {
                resource: resource.into(),
            },
            None,
            None,
        )?
        else {
            bail!("Provider did not return credentials")
        };
        for c in credentials {
            self.save_remote(id, c)?;
        }
        Ok(self
            .vault
            .list::<Credential>("credential")?
            .into_iter()
            .filter(|c| c.view.connection_id == id && c.view.resource == resource)
            .map(|c| c.view)
            .collect())
    }
    pub(crate) fn save_remote(
        &self,
        connection_id: &str,
        remote: RemoteCredential,
    ) -> Result<Credential> {
        crate::rotation::save_remote(&self.vault, connection_id, remote)
    }
    pub fn import(&self, input: ImportInput) -> Result<CredentialView> {
        ensure!(!input.name.trim().is_empty(), "Credential name is required");
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
        };
        let mut c = self.save_remote(&input.connection_id, remote)?;
        c.view.project = input.project;
        c.view.environment = input.environment;
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
        let c = self.save_remote(connection, remote)?;
        self.vault.event("credential.created", &c.view.id)?;
        Ok(c.view)
    }
    pub fn secret(&self, id: &str) -> Result<SecretValue> {
        let mut c: Credential = self.vault.get("credential", id)?;
        ensure!(!c.view.retired, "This credential is retired");
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
    pub fn save_binding(&self, mut binding: Binding) -> Result<Binding> {
        ensure!(
            valid_variable(&binding.variable),
            "Use a valid environment variable name, such as API_TOKEN"
        );
        ensure!(
            !binding.project.trim().is_empty() && !binding.environment.trim().is_empty(),
            "Project and environment are required"
        );
        let c: Credential = self.vault.get("credential", &binding.credential_id)?;
        ensure!(!c.view.retired, "Cannot bind a retired credential");
        ensure!(
            c.value
                .as_ref()
                .is_some_and(|v| v.fields.contains_key(&binding.field)),
            "Retrieve or import this credential before binding a field"
        );
        if let Destination::Provider { connection_id, .. } = &binding.destination {
            let connection: Connection = self.vault.get("connection", connection_id)?;
            ensure!(
                self.manifest(&connection.view.plugin_id)?
                    .operations
                    .contains(&OperationKind::WriteSecret),
                "Destination plugin cannot write secrets"
            );
        }
        ensure!(
            !self.vault.rotations()?.iter().any(|r| r.is_active()
                && (r.uses_credential(&binding.credential_id)
                    || r.bindings.iter().any(|b| b.id == binding.id))),
            "Finish the active rotation before editing this binding"
        );
        for other in self.vault.list::<Binding>("binding")? {
            ensure!(
                other.id == binding.id
                    || other.project != binding.project
                    || other.environment != binding.environment
                    || other.variable != binding.variable,
                "This project environment already has a binding for that variable"
            );
        }
        if binding.id.is_empty() {
            binding.id = new_id()?;
        }
        self.vault.put("binding", &binding.id, &binding)?;
        Ok(binding)
    }
    pub fn environment(
        &self,
        project: &str,
        environment: &str,
    ) -> Result<BTreeMap<String, String>> {
        let bindings = self.vault.list::<Binding>("binding")?;
        let mut values = BTreeMap::new();
        for b in bindings.iter().filter(|b| {
            b.project == project
                && b.environment == environment
                && matches!(b.destination, Destination::Local)
        }) {
            let secret = self.secret(&b.credential_id)?;
            let value = secret
                .fields
                .get(&b.field)
                .context("Bound credential field is unavailable")?;
            ensure!(
                values.insert(b.variable.clone(), value.clone()).is_none(),
                "Duplicate environment binding"
            );
        }
        ensure!(
            !values.is_empty(),
            "No local bindings match this project and environment"
        );
        Ok(values)
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
        let mut connection: Connection = self.vault.get("connection", id)?;
        connection.view.state = ConnectionState::Disconnected;
        connection.auth = AuthMaterial::Bearer {
            token: String::new(),
            refresh_token: None,
            expires_at: None,
            broker_registration: None,
        };
        self.vault.put("connection", id, &connection)?;
        self.vault.event("account.disconnected", id)
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
mod tests {
    use super::*;
    #[test]
    fn environment_names_cannot_inject_shell_syntax() {
        assert!(valid_variable("AWS_SECRET_ACCESS_KEY"));
        for s in ["", "2KEY", "KEY=VALUE", "A;B", "A\nB"] {
            assert!(!valid_variable(s));
        }
    }
}
