use crate::{
    App,
    agents::{self, AgentClient},
    model::*,
    prefs::Prefs,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, to_value};
use std::{collections::BTreeMap, path::Path};
use ts_rs::TS;

#[derive(Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Snapshot,
    Discover,
    ConnectLocal {
        source_id: String,
        connection_id: Option<String>,
    },
    OpenTokenPage {
        plugin_id: String,
    },
    ConnectToken {
        plugin_id: String,
        fields: BTreeMap<String, String>,
        connection_id: Option<String>,
    },
    Reconnect {
        connection_id: String,
    },
    Install {
        plugin_id: String,
        accept_permissions: bool,
    },
    Uninstall {
        plugin_id: String,
    },
    BeginAuth {
        plugin_id: String,
        config: BTreeMap<String, String>,
    },
    PollAuth {
        flow_id: String,
    },
    ChooseRole {
        flow_id: String,
        account_id: String,
        role: String,
    },
    CancelAuth {
        flow_id: String,
    },
    Resources {
        connection_id: String,
    },
    Sync {
        connection_id: String,
        resource: String,
    },
    ScanSources {
        plugin_id: Option<String>,
    },
    Dedicate {
        connection_id: String,
    },
    Delete {
        credential_id: String,
    },
    ConfirmDeleted {
        credential_id: String,
    },
    Import {
        input: ImportInput,
    },
    Create {
        connection_id: String,
        resource: String,
        name: String,
    },
    Reveal {
        credential_id: String,
    },
    Guide {
        connection_id: String,
        resource: String,
    },
    SaveBinding {
        binding: Binding,
    },
    RemoveBinding {
        binding_id: String,
    },
    PlanRotation {
        credential_id: String,
        name: String,
    },
    PlanReplacement {
        credential_id: String,
        replacement_id: String,
    },
    ConfirmManualRetirement {
        rotation_id: String,
    },
    AdvanceRotation {
        rotation_id: String,
    },
    VerifyBinding {
        rotation_id: String,
        binding_id: String,
    },
    ReconcileRotation {
        rotation_id: String,
        provider_id: String,
    },
    ReconcileRetirement {
        rotation_id: String,
    },
    AbortRotation {
        rotation_id: String,
    },
    Disconnect {
        connection_id: String,
    },
    Export {
        path: String,
        passphrase: String,
    },
    Check {
        credential_id: String,
    },
    SetGroups {
        credential_id: String,
        groups: Vec<String>,
    },
    SaveCustom {
        credential_id: Option<String>,
        name: String,
        fields: Vec<CustomField>,
    },
    SaveKey {
        credential_id: Option<String>,
        plugin_id: String,
        name: String,
        fields: BTreeMap<String, String>,
    },
    RotateNow {
        credential_id: String,
    },
    OpenVaultFolder,
    SetProjectFolders {
        folders: Vec<String>,
    },
    Restore {
        path: String,
        passphrase: String,
    },
    /// Values of the chosen fields by variable name. `via` names the client (such as
    /// `mcp.get_secret`) recorded in the activity log for each key read.
    Resolve {
        lines: Vec<EnvLine>,
        via: Option<String>,
    },
    /// The chosen fields as `.env` text.
    ExportEnv {
        lines: Vec<EnvLine>,
        via: Option<String>,
    },
    GetPrefs,
    SetPrefs {
        prefs: Prefs,
    },
    AgentStatus,
    /// Adds `getkeys mcp` to an agent app's MCP configuration.
    ConnectAgent {
        client: AgentClient,
    },
}
impl Command {
    /// Whether the command can change what the vault holds, so open windows reload.
    pub fn mutates(&self) -> bool {
        !matches!(
            self,
            Command::Snapshot
                | Command::Discover
                | Command::Reveal { .. }
                | Command::Resolve { .. }
                | Command::ExportEnv { .. }
                | Command::GetPrefs
                | Command::AgentStatus
                | Command::OpenTokenPage { .. }
                | Command::OpenVaultFolder
                | Command::Export { .. }
                | Command::Guide { .. }
                | Command::Resources { .. }
        )
    }
}
impl App {
    fn audit(&self, via: Option<String>, lines: &[EnvLine]) -> Result<()> {
        if let Some(via) = via {
            let ids: std::collections::BTreeSet<_> =
                lines.iter().map(|l| l.credential_id.as_str()).collect();
            for id in ids {
                self.vault.event(&via, id)?;
            }
        }
        Ok(())
    }
    pub fn dispatch(&self, command: Command) -> Result<Value> {
        Ok(match command {
            Command::Snapshot => to_value(self.snapshot()?)?,
            Command::Resolve { lines, via } => {
                let values = self.resolve(&lines)?;
                self.audit(via, &lines)?;
                to_value(values)?
            }
            Command::ExportEnv { lines, via } => {
                let text = self.export_env(&lines)?;
                self.audit(via, &lines)?;
                to_value(text)?
            }
            Command::GetPrefs => to_value(self.prefs()?)?,
            Command::SetPrefs { prefs } => to_value(self.set_prefs(prefs)?)?,
            Command::AgentStatus => to_value(agents::status()?)?,
            Command::ConnectAgent { client } => {
                agents::connect(client, &client.config()?, &agents::cli_path()?)?;
                to_value(agents::status()?)?
            }
            Command::Discover => to_value(self.discover()?)?,
            Command::ConnectLocal {
                source_id,
                connection_id,
            } => to_value(self.connect_local(&source_id, connection_id.as_deref())?)?,
            Command::OpenTokenPage { plugin_id } => {
                let url = self.manifest(&plugin_id)?.token_page;
                ensure!(
                    url.starts_with("https://"),
                    "This provider has no token page"
                );
                open::that_detached(url)?;
                Value::Null
            }
            Command::ConnectToken {
                plugin_id,
                fields,
                connection_id,
            } => to_value(self.connect_token(&plugin_id, fields, connection_id.as_deref())?)?,
            Command::PlanReplacement {
                credential_id,
                replacement_id,
            } => to_value(self.plan_replacement(&credential_id, &replacement_id)?)?,
            Command::ConfirmManualRetirement { rotation_id } => {
                to_value(self.confirm_manual_retirement(&rotation_id)?)?
            }
            Command::Reconnect { connection_id } => to_value(self.reconnect(&connection_id)?)?,
            Command::Install {
                plugin_id,
                accept_permissions,
            } => {
                self.install(&plugin_id, accept_permissions)?;
                Value::Null
            }
            Command::Uninstall { plugin_id } => {
                self.uninstall(&plugin_id)?;
                Value::Null
            }
            Command::BeginAuth { plugin_id, config } => {
                to_value(self.begin_auth(&plugin_id, config)?)?
            }
            Command::PollAuth { flow_id } => to_value(self.poll_auth(&flow_id)?)?,
            Command::ChooseRole {
                flow_id,
                account_id,
                role,
            } => to_value(self.choose_role(&flow_id, &account_id, &role)?)?,
            Command::CancelAuth { flow_id } => {
                self.vault.remove("auth_flow", &flow_id)?;
                Value::Null
            }
            Command::Resources { connection_id } => to_value(self.resources(&connection_id)?)?,
            Command::Sync {
                connection_id,
                resource,
            } => to_value(self.sync(&connection_id, &resource)?)?,
            Command::ScanSources { plugin_id } => {
                to_value(self.scan_sources(plugin_id.as_deref())?)?
            }
            Command::Dedicate { connection_id } => to_value(self.dedicate(&connection_id)?)?,
            Command::Delete { credential_id } => to_value(self.delete(&credential_id)?)?,
            Command::ConfirmDeleted { credential_id } => {
                self.confirm_deleted(&credential_id)?;
                Value::Null
            }
            Command::Import { input } => to_value(self.import(input)?)?,
            Command::Create {
                connection_id,
                resource,
                name,
            } => to_value(self.create(&connection_id, &resource, &name)?)?,
            Command::Reveal { credential_id } => to_value(self.secret(&credential_id)?)?,
            Command::Guide {
                connection_id,
                resource,
            } => to_value(self.guide(&connection_id, &resource)?)?,
            Command::SaveBinding { binding } => to_value(self.save_binding(binding)?)?,
            Command::RemoveBinding { binding_id } => {
                ensure!(
                    !self
                        .vault
                        .rotations()?
                        .iter()
                        .any(|r| r.is_active() && r.bindings.iter().any(|b| b.id == binding_id)),
                    "Finish the active rotation before removing this binding"
                );
                self.vault.remove("binding", &binding_id)?;
                Value::Null
            }
            Command::PlanRotation {
                credential_id,
                name,
            } => to_value(self.plan_rotation(&credential_id, &name)?)?,
            Command::AdvanceRotation { rotation_id } => {
                to_value(self.advance_rotation(&rotation_id)?)?
            }
            Command::VerifyBinding {
                rotation_id,
                binding_id,
            } => to_value(self.verify_binding(&rotation_id, &binding_id)?)?,
            Command::ReconcileRotation {
                rotation_id,
                provider_id,
            } => to_value(self.reconcile_rotation(&rotation_id, &provider_id)?)?,
            Command::ReconcileRetirement { rotation_id } => {
                to_value(self.reconcile_retirement(&rotation_id)?)?
            }
            Command::AbortRotation { rotation_id } => {
                self.abort_rotation(&rotation_id)?;
                Value::Null
            }
            Command::Disconnect { connection_id } => {
                self.disconnect(&connection_id)?;
                Value::Null
            }
            Command::Check { credential_id } => to_value(self.check(&credential_id)?)?,
            Command::SetGroups {
                credential_id,
                groups,
            } => to_value(self.set_groups(&credential_id, groups)?)?,
            Command::SaveCustom {
                credential_id,
                name,
                fields,
            } => to_value(self.save_custom(credential_id.as_deref(), &name, fields)?)?,
            Command::SaveKey {
                credential_id,
                plugin_id,
                name,
                fields,
            } => to_value(self.save_key(credential_id.as_deref(), &plugin_id, &name, fields)?)?,
            Command::RotateNow { credential_id } => to_value(self.rotate_now(&credential_id)?)?,
            Command::SetProjectFolders { folders } => to_value(self.set_project_folders(folders)?)?,
            Command::OpenVaultFolder => {
                let folder = self
                    .vault
                    .path()
                    .parent()
                    .context("The vault folder is unavailable")?;
                open::that_detached(folder)?;
                Value::Null
            }
            Command::Export { path, passphrase } => {
                self.vault.export(Path::new(&path), passphrase)?;
                Value::Null
            }
            Command::Restore { path, passphrase } => {
                self.vault.import_backup(Path::new(&path), passphrase)?;
                Value::Null
            }
        })
    }
}
