//! A new project made while creating a key, for providers whose keys belong to a project.
use crate::{App, api::*, app::valid_variable, model::*, plugins::OperationError};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Serialize, Deserialize, TS)]
pub struct Created {
    pub resource: Resource,
    /// A secret GetKeys made up for it, such as a database password, saved as a key.
    pub generated: Option<CredentialView>,
}

impl App {
    /// The account's choices for a `listed` option of a new resource, such as its IAM policies.
    pub fn choices(&self, connection_id: &str, option: &str) -> Result<Vec<Choice>> {
        let Outcome::Choices(choices) = self.execute(
            connection_id,
            Operation::ListChoices {
                option: option.into(),
            },
            None,
            None,
        )?
        else {
            bail!("Provider did not return its choices")
        };
        Ok(choices)
    }
    /// The provider's page for making a new resource by hand, for providers whose API can't.
    pub fn resource_page(&self, plugin_id: &str) -> Result<String> {
        let manifest = self.manifest(plugin_id)?;
        let page = manifest
            .new_resource
            .map(|r| r.page)
            .filter(|p| p.starts_with("https://"))
            .unwrap_or_else(|| manifest.homepage.clone());
        ensure!(
            page.starts_with("https://"),
            "{} has no page for this",
            manifest.name
        );
        Ok(page)
    }
    /// Accounts and organisations a connection can make new resources in.
    pub fn owners(&self, connection_id: &str) -> Result<Vec<Owner>> {
        let Outcome::Owners(owners) =
            self.execute(connection_id, Operation::ListOwners, None, None)?
        else {
            bail!("Provider did not return its accounts")
        };
        ensure!(
            !owners.is_empty(),
            "This account can't create anything new. Check its permissions."
        );
        Ok(owners)
    }
    /// Creates a repository or project. A secret the provider asks GetKeys to make up is saved
    /// first, so it isn't lost if the provider made the resource but the reply went missing.
    pub fn create_resource(
        &self,
        connection_id: &str,
        owner: &str,
        name: &str,
        options: BTreeMap<String, String>,
    ) -> Result<Created> {
        let name = name.trim();
        ensure!(
            !name.is_empty() && name.chars().count() <= 100,
            "Give it a name under 100 characters"
        );
        let connection: Connection = self.vault.get("connection", connection_id)?;
        let manifest = self.manifest(&connection.view.plugin_id)?;
        let template = manifest
            .new_resource
            .as_ref()
            .filter(|_| manifest.operations.contains(&OperationKind::CreateResource))
            .with_context(|| format!("{} can't create anything from GetKeys", manifest.name))?;
        let generated = match &template.generated {
            Some(field) => {
                let value = password()?;
                let key = self.save_custom(
                    None,
                    &format!("{name} {}", field.label.to_lowercase()),
                    vec![CustomField {
                        variable: if valid_variable(&field.env) {
                            field.env.clone()
                        } else {
                            "PASSWORD".into()
                        },
                        value: value.clone(),
                        secret: true,
                    }],
                )?;
                Some((key, value))
            }
            None => None,
        };
        let result = self.execute(
            connection_id,
            Operation::CreateResource {
                owner: owner.into(),
                name: name.into(),
                options,
            },
            None,
            generated.as_ref().map(|(_, value)| value.as_str()),
        );
        let resource = match result {
            Ok(Outcome::Resource(resource)) => resource,
            Ok(_) => bail!(
                "{} did not confirm the new {}",
                manifest.name,
                template.noun
            ),
            Err(error) => {
                if let Some((key, _)) = &generated {
                    let uncertain = error
                        .downcast_ref::<OperationError>()
                        .is_some_and(|e| e.may_have_changed);
                    if uncertain {
                        return Err(error.context(format!(
                            "Check {} before trying again. Its {} is saved as “{}”.",
                            manifest.name,
                            template
                                .generated
                                .as_ref()
                                .map_or("secret".into(), |f| f.label.to_lowercase()),
                            key.name
                        )));
                    }
                    self.vault.remove("credential", &key.id)?;
                }
                return Err(error);
            }
        };
        self.vault.event(
            "resource.created",
            &format!("{} {}", manifest.name, resource.name),
        )?;
        Ok(Created {
            resource,
            generated: generated.map(|(key, _)| key),
        })
    }
}

/// A random 32-character password of letters and digits, which every provider accepts.
fn password() -> Result<String> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    loop {
        let mut out = String::new();
        while out.len() < 32 {
            let mut bytes = [0u8; 64];
            getrandom::fill(&mut bytes)
                .map_err(|_| anyhow::anyhow!("Operating system randomness is unavailable"))?;
            // Bytes past the largest multiple of the alphabet are skipped so each is as likely.
            for b in bytes.into_iter().filter(|b| *b < 248) {
                if out.len() < 32 {
                    out.push(ALPHABET[usize::from(b) % ALPHABET.len()] as char);
                }
            }
        }
        // At least one of each kind, for providers with password rules.
        if out.bytes().any(|b| b.is_ascii_uppercase())
            && out.bytes().any(|b| b.is_ascii_lowercase())
            && out.bytes().any(|b| b.is_ascii_digit())
        {
            return Ok(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passwords_are_long_and_varied() {
        let (a, b) = (password().unwrap(), password().unwrap());
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert_ne!(a, b);
    }
}
