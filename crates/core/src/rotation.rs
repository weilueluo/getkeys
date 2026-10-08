use crate::{App, api::*, model::*, new_id, now, vault::Vault};
use anyhow::{Context, Result, bail, ensure};

pub fn save_remote(
    vault: &Vault,
    connection_id: &str,
    remote: RemoteCredential,
) -> Result<Credential> {
    let previous = vault
        .list::<Credential>("credential")?
        .into_iter()
        .find(|c| c.view.connection_id == connection_id && c.view.provider_id == remote.id);
    let value = remote
        .value
        .or_else(|| previous.as_ref().and_then(|c| c.value.clone()));
    let view = CredentialView {
        id: previous
            .as_ref()
            .map(|c| c.view.id.clone())
            .unwrap_or(new_id()?),
        connection_id: connection_id.into(),
        provider_id: remote.id,
        name: remote.name,
        credential_type: remote.credential_type,
        resource: previous
            .as_ref()
            .filter(|c| c.view.can_rotate)
            .map(|c| c.view.resource.clone())
            .unwrap_or(remote.resource),
        project: previous
            .as_ref()
            .map(|c| c.view.project.clone())
            .unwrap_or_default(),
        environment: previous
            .as_ref()
            .map(|c| c.view.environment.clone())
            .unwrap_or_else(|| "development".into()),
        created_at: remote.created_at,
        expires_at: remote.expires_at,
        value_available: value.is_some(),
        can_rotate: remote.can_rotate || previous.as_ref().is_some_and(|c| c.view.can_rotate),
        retired: previous.as_ref().is_some_and(|c| c.view.retired),
    };
    let c = Credential { view, value };
    vault.put("credential", &c.view.id, &c)?;
    Ok(c)
}

impl App {
    pub fn plan_rotation(&self, credential_id: &str, name: &str) -> Result<Rotation> {
        let c: Credential = self.vault.get("credential", credential_id)?;
        ensure!(
            c.view.can_rotate && !c.view.retired,
            "This credential requires guided replacement; its scope cannot be cloned automatically"
        );
        ensure!(!name.trim().is_empty(), "Replacement name is required");
        let connection: Connection = self.vault.get("connection", &c.view.connection_id)?;
        let manifest = self.manifest(&connection.view.plugin_id)?;
        ensure!(
            manifest
                .operations
                .contains(&OperationKind::CreateCredential)
                && manifest
                    .operations
                    .contains(&OperationKind::RevokeCredential),
            "This provider does not support automated rotation"
        );
        let r = self.rotation_plan(&c, name, manifest.version)?;
        self.vault.save_rotation(&r)?;
        self.vault.event("rotation.planned", &r.id)?;
        Ok(r)
    }
    pub fn plan_replacement(&self, credential_id: &str, replacement_id: &str) -> Result<Rotation> {
        let old: Credential = self.vault.get("credential", credential_id)?;
        let candidate: Credential = self.vault.get("credential", replacement_id)?;
        ensure!(
            !old.view.retired && !candidate.view.retired && credential_id != replacement_id,
            "Choose an active, different replacement credential"
        );
        ensure!(
            old.view.connection_id == candidate.view.connection_id
                && old.view.resource == candidate.view.resource,
            "The replacement must belong to the same account and resource"
        );
        ensure!(
            !old.value
                .as_ref()
                .zip(candidate.value.as_ref())
                .is_some_and(|(a, b)| a.fields == b.fields),
            "The replacement contains the same secret as the old credential"
        );
        let connection: Connection = self.vault.get("connection", &old.view.connection_id)?;
        let manifest = self.manifest(&connection.view.plugin_id)?;
        ensure!(
            manifest.operations.contains(&OperationKind::Guide),
            "This provider uses automatic replacement"
        );
        self.execute(
            &old.view.connection_id,
            Operation::ValidateCredential {
                resource: old.view.resource.clone(),
            },
            Some(
                candidate
                    .value
                    .as_ref()
                    .context("Import the replacement value first")?,
            ),
            None,
        )?;
        let mut r = self.rotation_plan(&old, &candidate.view.name, manifest.version)?;
        r.candidate_id = Some(replacement_id.into());
        r.state = RotationState::Stored;
        self.vault.save_rotation(&r)?;
        self.vault.event("rotation.replacement_selected", &r.id)?;
        Ok(r)
    }
    fn rotation_plan(&self, c: &Credential, name: &str, version: String) -> Result<Rotation> {
        Ok(Rotation {
            id: new_id()?,
            credential_id: c.view.id.clone(),
            connection_id: c.view.connection_id.clone(),
            resource: c.view.resource.clone(),
            plugin_version: version,
            name: name.into(),
            state: RotationState::Planned,
            candidate_id: None,
            bindings: self
                .vault
                .list::<Binding>("binding")?
                .into_iter()
                .filter(|b| b.credential_id == c.view.id)
                .collect(),
            updated_bindings: Vec::new(),
            verified_bindings: Vec::new(),
            created_at: now(),
        })
    }
    pub fn confirm_manual_retirement(&self, id: &str) -> Result<Rotation> {
        let mut r = self.vault.rotation(id)?;
        ensure!(
            r.state == RotationState::ReadyToRetire,
            "Verify consumers before confirming provider-side revocation"
        );
        let c: Connection = self.vault.get("connection", &r.connection_id)?;
        let manifest = self.manifest(&c.view.plugin_id)?;
        ensure!(
            !manifest
                .operations
                .contains(&OperationKind::RevokeCredential)
                && manifest.operations.contains(&OperationKind::Guide),
            "Use automated retirement for this provider"
        );
        let candidate: Credential = self.vault.get(
            "credential",
            r.candidate_id
                .as_deref()
                .context("Replacement is missing")?,
        )?;
        self.execute(
            &r.connection_id,
            Operation::ValidateCredential {
                resource: r.resource.clone(),
            },
            candidate.value.as_ref(),
            None,
        )?;
        let old: Credential = self.vault.get("credential", &r.credential_id)?;
        let tx = self.vault.db.unchecked_transaction()?;
        ensure!(
            self.vault.rotation(id)?.state == RotationState::ReadyToRetire,
            "Rotation changed. Refresh before continuing."
        );
        mark_retired(&self.vault, &old)?;
        r.state = RotationState::Complete;
        self.vault.save_rotation(&r)?;
        self.vault
            .event("rotation.manual_revocation_confirmed", id)?;
        tx.commit()?;
        Ok(r)
    }
    pub fn advance_rotation(&self, id: &str) -> Result<Rotation> {
        let r = self.vault.rotation(id)?;
        let c: Connection = self.vault.get("connection", &r.connection_id)?;
        ensure!(
            self.manifest(&c.view.plugin_id)?.version == r.plugin_version,
            "Restore the rotation's plugin version before continuing"
        );
        match r.state {
            RotationState::Planned => create(&self.vault, r, |op| {
                self.execute(&c.view.id, op, None, None)
            }),
            RotationState::Stored | RotationState::Distributing => {
                distribute(&self.vault, r, |connection, op, candidate, injected| {
                    self.execute(connection, op, candidate, injected)
                })
            }
            RotationState::ReadyToRetire | RotationState::Deactivated => {
                ensure!(
                    self.manifest(&c.view.plugin_id)?
                        .operations
                        .contains(&OperationKind::RevokeCredential),
                    "Revoke this token in the provider dashboard, then confirm manual retirement"
                );
                retire(
                    &self.vault,
                    r,
                    self.manifest(&c.view.plugin_id)?
                        .operations
                        .contains(&OperationKind::DeactivateCredential),
                    |op, candidate| self.execute(&c.view.id, op, candidate, None),
                )
            }
            _ => bail!(
                "This rotation is waiting for verification or reconciliation; no provider mutation was retried"
            ),
        }
    }
    pub fn verify_binding(&self, id: &str, binding_id: &str) -> Result<Rotation> {
        let mut r = self.vault.rotation(id)?;
        ensure!(
            r.state == RotationState::AwaitingVerification,
            "This rotation is not waiting for consumer verification"
        );
        ensure!(
            r.updated_bindings.iter().any(|b| b == binding_id),
            "This consumer has not been updated"
        );
        if !r.verified_bindings.contains(&binding_id.into()) {
            r.verified_bindings.push(binding_id.into());
        }
        let next = if r
            .bindings
            .iter()
            .all(|b| r.verified_bindings.contains(&b.id))
        {
            RotationState::ReadyToRetire
        } else {
            RotationState::AwaitingVerification
        };
        self.vault
            .transition(&mut r, &RotationState::AwaitingVerification, next)?;
        self.vault.event("consumer.verified", binding_id)?;
        Ok(r)
    }
    pub fn reconcile_rotation(&self, id: &str, provider_id: &str) -> Result<Rotation> {
        let mut r = self.vault.rotation(id)?;
        ensure!(
            matches!(
                r.state,
                RotationState::Creating | RotationState::Uncertain { .. }
            ),
            "Only an uncertain creation can be reconciled with a replacement ID"
        );
        if let RotationState::Uncertain { action } = &r.state {
            ensure!(
                action == "create",
                "This rotation needs retirement reconciliation"
            );
        }
        let old: Credential = self.vault.get("credential", &r.credential_id)?;
        ensure!(
            provider_id != old.view.provider_id,
            "The replacement cannot be the old credential"
        );
        self.sync(&r.connection_id, &r.resource)?;
        let candidate = self
            .vault
            .list::<Credential>("credential")?
            .into_iter()
            .find(|c| {
                c.view.connection_id == r.connection_id
                    && c.view.provider_id == provider_id
                    && c.view.resource == r.resource
            })
            .context("Replacement ID was not found. Inspect the provider inventory first.")?;
        self.secret(&candidate.view.id).context(
            "Import the replacement value before reconciling; the provider may only reveal it once",
        )?;
        let expected = r.state.clone();
        r.candidate_id = Some(candidate.view.id);
        self.vault
            .transition(&mut r, &expected, RotationState::Stored)?;
        Ok(r)
    }
    pub fn reconcile_retirement(&self, id: &str) -> Result<Rotation> {
        let mut r = self.vault.rotation(id)?;
        let expected = r.state.clone();
        let action = match &r.state {
            RotationState::Retiring => "revoke",
            RotationState::Deactivating => "deactivate",
            RotationState::Uncertain { action } => action.as_str(),
            _ => bail!("Retirement is not uncertain"),
        };
        ensure!(
            action != "create",
            "Reconcile creation with the replacement's provider ID"
        );
        if action == "deactivate" {
            self.vault
                .transition(&mut r, &expected, RotationState::ReadyToRetire)?;
            return Ok(r);
        }
        let old: Credential = self.vault.get("credential", &r.credential_id)?;
        let Outcome::Credentials(keys) = self.execute(
            &r.connection_id,
            Operation::ListCredentials {
                resource: r.resource.clone(),
            },
            None,
            None,
        )?
        else {
            bail!("Provider did not return inventory")
        };
        let next = if keys.iter().any(|c| c.id == old.view.provider_id) {
            RotationState::ReadyToRetire
        } else {
            mark_retired(&self.vault, &old)?;
            RotationState::Complete
        };
        self.vault.transition(&mut r, &expected, next)?;
        Ok(r)
    }
    pub fn abort_rotation(&self, id: &str) -> Result<()> {
        let mut r = self.vault.rotation(id)?;
        self.vault
            .transition(&mut r, &RotationState::Planned, RotationState::Aborted)
    }
}

fn uncertain(vault: &Vault, r: &mut Rotation, action: &str) -> Result<()> {
    r.state = RotationState::Uncertain {
        action: action.into(),
    };
    vault.save_rotation(r)
}

pub fn create(
    vault: &Vault,
    mut r: Rotation,
    mut execute: impl FnMut(Operation) -> Result<Outcome>,
) -> Result<Rotation> {
    vault.transition(&mut r, &RotationState::Planned, RotationState::Creating)?;
    let result = execute(Operation::CreateCredential {
        resource: r.resource.clone(),
        name: r.name.clone(),
    });
    let remote = match result {
        Ok(Outcome::Credential(c)) if c.value.is_some() => c,
        Ok(_) => {
            uncertain(vault, &mut r, "create")?;
            bail!("Creation returned no usable value. Inspect the provider before reconciling.");
        }
        Err(error) => {
            if error
                .downcast_ref::<crate::plugins::OperationError>()
                .is_some_and(|e| e.may_have_changed)
            {
                uncertain(vault, &mut r, "create")?;
            } else {
                vault.transition(&mut r, &RotationState::Creating, RotationState::Planned)?;
            }
            return Err(error);
        }
    };
    let tx = vault.db.unchecked_transaction()?;
    let c = save_remote(vault, &r.connection_id, remote)?;
    r.candidate_id = Some(c.view.id);
    r.state = RotationState::Stored;
    vault.save_rotation(&r)?;
    vault.event("rotation.created", &r.id)?;
    tx.commit()?;
    Ok(r)
}

pub fn distribute(
    vault: &Vault,
    mut r: Rotation,
    mut execute: impl FnMut(&str, Operation, Option<&SecretValue>, Option<&str>) -> Result<Outcome>,
) -> Result<Rotation> {
    ensure!(
        matches!(r.state, RotationState::Stored | RotationState::Distributing),
        "Replacement is not ready to distribute"
    );
    let c: Credential = vault.get(
        "credential",
        r.candidate_id
            .as_deref()
            .context("Replacement is missing")?,
    )?;
    let value = c.value.as_ref().context("Replacement value is missing")?;
    execute(
        &r.connection_id,
        Operation::ValidateCredential {
            resource: r.resource.clone(),
        },
        Some(value),
        None,
    )?;
    let expected = r.state.clone();
    vault.transition(&mut r, &expected, RotationState::Distributing)?;
    for binding in r.bindings.clone() {
        if r.updated_bindings.contains(&binding.id) {
            continue;
        }
        let secret = value
            .fields
            .get(&binding.field)
            .context("Replacement does not contain a bound field")?;
        if let Destination::Provider {
            connection_id,
            resource,
        } = &binding.destination
        {
            execute(
                connection_id,
                Operation::WriteSecret {
                    resource: resource.clone(),
                    name: binding.variable.clone(),
                    environment: binding.environment.clone(),
                },
                None,
                Some(secret),
            )?;
        }
        let mut updated = binding.clone();
        updated.credential_id = c.view.id.clone();
        let tx = vault.db.unchecked_transaction()?;
        vault.put("binding", &binding.id, &updated)?;
        r.updated_bindings.push(binding.id);
        vault.save_rotation(&r)?;
        tx.commit()?;
    }
    let next = if r.bindings.is_empty() {
        RotationState::ReadyToRetire
    } else {
        RotationState::AwaitingVerification
    };
    vault.transition(&mut r, &RotationState::Distributing, next)?;
    Ok(r)
}

pub fn retire(
    vault: &Vault,
    mut r: Rotation,
    can_deactivate: bool,
    mut execute: impl FnMut(Operation, Option<&SecretValue>) -> Result<Outcome>,
) -> Result<Rotation> {
    ensure!(
        matches!(
            r.state,
            RotationState::ReadyToRetire | RotationState::Deactivated
        ),
        "Verify all consumers before retiring the old credential"
    );
    let candidate: Credential = vault.get(
        "credential",
        r.candidate_id
            .as_deref()
            .context("Replacement is missing")?,
    )?;
    execute(
        Operation::ValidateCredential {
            resource: r.resource.clone(),
        },
        Some(
            candidate
                .value
                .as_ref()
                .context("Replacement value is missing")?,
        ),
    )?;
    let old: Credential = vault.get("credential", &r.credential_id)?;
    let deactivate = can_deactivate && r.state == RotationState::ReadyToRetire;
    let expected = r.state.clone();
    vault.transition(
        &mut r,
        &expected,
        if deactivate {
            RotationState::Deactivating
        } else {
            RotationState::Retiring
        },
    )?;
    let op = if deactivate {
        Operation::DeactivateCredential {
            resource: r.resource.clone(),
            credential: old.view.provider_id.clone(),
        }
    } else {
        Operation::RevokeCredential {
            resource: r.resource.clone(),
            credential: old.view.provider_id.clone(),
        }
    };
    if let Err(error) = execute(op, None) {
        uncertain(
            vault,
            &mut r,
            if deactivate { "deactivate" } else { "revoke" },
        )?;
        return Err(error);
    }
    if !deactivate {
        mark_retired(vault, &old)?;
    }
    r.state = if deactivate {
        RotationState::Deactivated
    } else {
        RotationState::Complete
    };
    vault.save_rotation(&r)?;
    vault.event("rotation.retired", &r.id)?;
    Ok(r)
}
fn mark_retired(vault: &Vault, old: &Credential) -> Result<()> {
    let mut old = old.clone();
    old.view.retired = true;
    old.view.value_available = false;
    old.value = None;
    vault.put("credential", &old.view.id, &old)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn fixture() -> (tempfile::TempDir, Vault, Rotation) {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(&dir.path().join("vault.db"), &[1; 32]).unwrap();
        let c = save_remote(&vault, "connection", remote("old")).unwrap();
        let binding = Binding {
            id: "binding".into(),
            credential_id: c.view.id.clone(),
            field: "token".into(),
            variable: "TOKEN".into(),
            project: "app".into(),
            environment: "production".into(),
            destination: Destination::Local,
        };
        vault.put("binding", &binding.id, &binding).unwrap();
        let r = Rotation {
            id: "rotation".into(),
            credential_id: c.view.id,
            connection_id: "connection".into(),
            resource: "project".into(),
            plugin_version: "1".into(),
            name: "replacement".into(),
            state: RotationState::Planned,
            candidate_id: None,
            bindings: vec![binding],
            updated_bindings: vec![],
            verified_bindings: vec![],
            created_at: 0,
        };
        vault.save_rotation(&r).unwrap();
        (dir, vault, r)
    }
    fn remote(id: &str) -> RemoteCredential {
        RemoteCredential {
            id: id.into(),
            name: id.into(),
            credential_type: "token".into(),
            resource: "project".into(),
            created_at: None,
            expires_at: None,
            value: Some(SecretValue {
                fields: BTreeMap::from([("token".into(), format!("secret-{id}"))]),
            }),
            can_rotate: true,
        }
    }
    #[test]
    fn uncertain_creation_is_never_retried() {
        let (_dir, vault, r) = fixture();
        let mut calls = 0;
        assert!(
            create(&vault, r, |_| {
                calls += 1;
                Err(crate::plugins::OperationError {
                    message: "connection lost after write".into(),
                    may_have_changed: true,
                }
                .into())
            })
            .is_err()
        );
        let r = vault.rotation("rotation").unwrap();
        assert!(matches!(r.state, RotationState::Uncertain { .. }));
        assert!(
            create(&vault, r, |_| {
                calls += 1;
                Ok(Outcome::Credential(remote("new")))
            })
            .is_err()
        );
        assert_eq!(calls, 1);
    }
    #[test]
    fn replacement_is_saved_and_consumers_must_be_verified_before_retirement() {
        let (_dir, vault, r) = fixture();
        let r = create(&vault, r, |_| Ok(Outcome::Credential(remote("new")))).unwrap();
        assert!(
            vault
                .get::<Credential>("credential", r.candidate_id.as_ref().unwrap())
                .unwrap()
                .value
                .is_some()
        );
        let r = distribute(&vault, r, |_, _, _, _| Ok(Outcome::Done)).unwrap();
        assert_eq!(r.state, RotationState::AwaitingVerification);
        let mut calls = 0;
        assert!(
            retire(&vault, r, true, |_, _| {
                calls += 1;
                Ok(Outcome::Done)
            })
            .is_err()
        );
        assert_eq!(calls, 0);
    }
    #[test]
    fn failure_before_a_write_keeps_the_plan_retryable() {
        let (_dir, vault, r) = fixture();
        assert!(create(&vault, r, |_| bail!("connection could not be refreshed")).is_err());
        assert_eq!(
            vault.rotation("rotation").unwrap().state,
            RotationState::Planned
        );
    }
    #[test]
    fn concurrent_rotations_for_one_resource_are_rejected() {
        let (_dir, vault, mut r) = fixture();
        r.id = "second".into();
        assert!(vault.save_rotation(&r).is_err());
    }
    #[test]
    fn metadata_refresh_does_not_erase_a_one_time_secret() {
        let (_dir, vault, _) = fixture();
        let mut c = remote("old");
        c.value = None;
        assert!(
            save_remote(&vault, "connection", c)
                .unwrap()
                .value
                .is_some()
        );
    }
    #[test]
    fn failed_consumer_update_preserves_old_key_and_resumes_distribution() {
        let (_dir, vault, mut r) = fixture();
        r.bindings[0].destination = Destination::Provider {
            connection_id: "destination".into(),
            resource: "repo".into(),
        };
        vault.save_rotation(&r).unwrap();
        let r = create(&vault, r, |_| Ok(Outcome::Credential(remote("new")))).unwrap();
        assert!(
            distribute(&vault, r, |_, op, _, _| {
                if matches!(op, Operation::WriteSecret { .. }) {
                    bail!("consumer unavailable");
                }
                Ok(Outcome::Done)
            })
            .is_err()
        );
        let r = vault.rotation("rotation").unwrap();
        assert_eq!(r.state, RotationState::Distributing);
        let old: Credential = vault.get("credential", &r.credential_id).unwrap();
        assert!(!old.view.retired && old.value.is_some());
        let r = distribute(&vault, r, |_, _, _, _| Ok(Outcome::Done)).unwrap();
        assert_eq!(r.state, RotationState::AwaitingVerification);
        assert_eq!(r.updated_bindings, ["binding"]);
    }
    #[test]
    fn verified_rotation_deactivates_before_deleting_and_erases_old_value() {
        let (dir, vault, r) = fixture();
        let r = create(&vault, r, |_| Ok(Outcome::Credential(remote("new")))).unwrap();
        let r = distribute(&vault, r, |_, _, _, _| Ok(Outcome::Done)).unwrap();
        let app = App {
            vault,
            registry: crate::plugins::Registry::open(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../registry/dist"),
                dir.path().join("plugins"),
            )
            .unwrap(),
        };
        let r = app.verify_binding(&r.id, "binding").unwrap();
        assert_eq!(r.state, RotationState::ReadyToRetire);
        let mut mutations = Vec::new();
        let r = retire(&app.vault, r, true, |op, _| {
            if let Operation::DeactivateCredential { credential, .. } = op {
                mutations.push(credential);
            }
            Ok(Outcome::Done)
        })
        .unwrap();
        assert_eq!(r.state, RotationState::Deactivated);
        assert_eq!(mutations, ["old"]);
        let r = retire(&app.vault, r, true, |op, _| {
            if let Operation::RevokeCredential { credential, .. } = op {
                mutations.push(credential);
            }
            Ok(Outcome::Done)
        })
        .unwrap();
        assert_eq!(r.state, RotationState::Complete);
        assert_eq!(mutations, ["old", "old"]);
        let old: Credential = app.vault.get("credential", &r.credential_id).unwrap();
        assert!(old.view.retired && old.value.is_none());
        assert_eq!(
            app.environment("app", "production").unwrap()["TOKEN"],
            "secret-new"
        );
    }
    #[test]
    fn active_rotations_freeze_source_values_consumers_and_destination_plugins() {
        let (dir, vault, mut r) = fixture();
        r.bindings[0].destination = Destination::Provider {
            connection_id: "destination".into(),
            resource: "repo".into(),
        };
        vault.save_rotation(&r).unwrap();
        vault
            .put(
                "connection",
                "destination",
                &Connection {
                    view: ConnectionView {
                        id: "destination".into(),
                        plugin_id: "official.github".into(),
                        name: "Destination".into(),
                        state: ConnectionState::Connected,
                        config: BTreeMap::new(),
                    },
                    identity_id: "test".into(),
                    auth: AuthMaterial::Bearer {
                        token: "fixture".into(),
                        refresh_token: None,
                        expires_at: None,
                        broker_registration: None,
                    },
                },
            )
            .unwrap();
        let app = App {
            vault,
            registry: crate::plugins::Registry::open(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../registry/dist"),
                dir.path().join("plugins"),
            )
            .unwrap(),
        };
        assert!(
            app.install("official.github", false)
                .unwrap_err()
                .to_string()
                .contains("active rotations")
        );
        assert!(
            app.disconnect("destination")
                .unwrap_err()
                .to_string()
                .contains("active rotations")
        );
        let mut new_binding = r.bindings[0].clone();
        new_binding.id.clear();
        new_binding.destination = Destination::Local;
        assert!(
            app.save_binding(new_binding)
                .unwrap_err()
                .to_string()
                .contains("active rotation")
        );
        assert!(
            app.import(ImportInput {
                connection_id: "connection".into(),
                provider_id: Some("old".into()),
                resource: "project".into(),
                name: "overwrite".into(),
                project: "app".into(),
                environment: "production".into(),
                fields: BTreeMap::from([("token".into(), "changed".into())]),
            })
            .unwrap_err()
            .to_string()
            .contains("active rotation")
        );
    }
}
