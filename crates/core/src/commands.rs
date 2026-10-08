use crate::{App, model::*};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, to_value};
use std::{collections::BTreeMap, path::Path};
use ts_rs::TS;

#[derive(Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Snapshot,
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
    Restore {
        path: String,
        passphrase: String,
    },
}
impl App {
    pub fn dispatch(&self, command: Command) -> Result<Value> {
        Ok(match command {
            Command::Snapshot => to_value(self.snapshot()?)?,
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
