use getkeys_plugin_api::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const MANIFEST: &str = include_str!("../manifest.json");

pub fn handle(call: Call) -> Step {
    execute(call).unwrap_or_else(|_| {
        error(
            "invalid_response",
            "Supabase returned an unexpected response.",
        )
    })
}

fn credential(v: &Value, resource: &str) -> Result<RemoteCredential, &'static str> {
    let kind = v["type"].as_str().ok_or("missing key type")?;
    let value = v["api_key"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| SecretValue {
            fields: BTreeMap::from([("token".into(), s.into())]),
        });
    Ok(RemoteCredential {
        id: v["id"].as_str().ok_or("missing key id")?.into(),
        name: key_name(kind, v["name"].as_str().unwrap_or_default()),
        credential_type: kind.into(),
        resource: resource.into(),
        created_at: v["inserted_at"].as_str().map(str::to_owned),
        expires_at: None,
        value,
        can_rotate: kind == "secret",
        details: BTreeMap::from([("url".into(), format!("https://{resource}.supabase.co"))]),
    })
}

/// Every project has a publishable and a secret key both named "default", so the name says
/// which one it is.
fn key_name(kind: &str, name: &str) -> String {
    let kind_label = match kind {
        "publishable" => "Publishable key",
        "secret" => "Secret key",
        _ => "",
    };
    match (kind, name) {
        ("legacy", "anon") => "Legacy anon key".into(),
        ("legacy", "service_role") => "Legacy service_role key".into(),
        ("legacy", name) => format!("Legacy {name} key"),
        (_, "" | "default") if !kind_label.is_empty() => kind_label.into(),
        (_, name) if !kind_label.is_empty() => format!("{name} · {}", kind_label.to_lowercase()),
        (kind, "") => kind.into(),
        (_, name) => name.into(),
    }
}

fn project(p: &Value) -> Result<Resource, &'static str> {
    let id = p["ref"]
        .as_str()
        .or(p["id"].as_str())
        .ok_or("project ref")?;
    Ok(Resource {
        id: id.into(),
        name: p["name"].as_str().ok_or("project name")?.into(),
        kind: "project".into(),
        url: format!("https://supabase.com/dashboard/project/{id}"),
        pending: p["status"] == "COMING_UP",
        note: String::new(),
    })
}

/// A project in organisation `owner`. GetKeys makes up its database password and puts it in
/// the request, so this code never sees it.
fn create(call: &Call) -> Result<Step, &'static str> {
    let Operation::CreateResource {
        owner,
        name,
        options,
    } = &call.operation
    else {
        return Err("operation");
    };
    let Some(response) = &call.response else {
        if owner.is_empty() {
            return Ok(error(
                "invalid_owner",
                "Choose the Supabase organization for the project.",
            ));
        }
        let region = options.get("region").map_or("americas", String::as_str);
        let mut r = post(
            "https://api.supabase.com/v1/projects".into(),
            json!({
                "name": name,
                "organization_slug": owner,
                "region_selection": {"type": "smartGroup", "code": region},
            }),
        );
        r.secret = Some(SecretInjection::JsonField {
            field: "db_pass".into(),
        });
        return Ok(request(r, Value::Null));
    };
    if let Some(err) = name_error(response, "project").or_else(|| provider_error(response)) {
        return Ok(err);
    }
    let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
    Ok(done(Outcome::Resource(project(&v)?)))
}

fn execute(call: Call) -> Result<Step, &'static str> {
    if matches!(call.operation, Operation::CreateResource { .. }) {
        return create(&call);
    }
    if let Some(response) = call.response {
        if let Some(err) = provider_error(&response) {
            return Ok(err);
        }
        if matches!(
            call.operation,
            Operation::RevokeCredential { .. } | Operation::ValidateCredential { .. }
        ) {
            return Ok(done(Outcome::Done));
        }
        let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
        return Ok(match call.operation {
            Operation::Identity => {
                let orgs = v.as_array().ok_or("organizations")?;
                let names: Vec<_> = orgs.iter().filter_map(|o| o["name"].as_str()).collect();
                let mut ids: Vec<_> = orgs.iter().filter_map(|o| o["id"].as_str()).collect();
                ids.sort_unstable();
                done(Outcome::Identity(Identity {
                    id: ids.join(","),
                    name: if names.is_empty() {
                        "Supabase account".into()
                    } else {
                        names.join(", ")
                    },
                }))
            }
            Operation::ListResources => done(Outcome::Resources(
                v.as_array()
                    .ok_or("projects")?
                    .iter()
                    .map(project)
                    .collect::<Result<_, &'static str>>()?,
            )),
            Operation::ListOwners => done(Outcome::Owners(
                v.as_array()
                    .ok_or("organizations")?
                    .iter()
                    .map(|o| {
                        Ok(Owner {
                            id: o["slug"]
                                .as_str()
                                .or(o["id"].as_str())
                                .ok_or("organization slug")?
                                .into(),
                            name: o["name"].as_str().ok_or("organization name")?.into(),
                        })
                    })
                    .collect::<Result<_, &'static str>>()?,
            )),
            Operation::ListCredentials { resource } => {
                let keys = v
                    .as_array()
                    .ok_or("keys")?
                    .iter()
                    .map(|k| credential(k, &resource))
                    .collect::<Result<_, &'static str>>()?;
                done(Outcome::Credentials(keys))
            }
            Operation::ReadCredential {
                resource,
                credential: id,
            } => {
                let key = v
                    .as_array()
                    .ok_or("keys")?
                    .iter()
                    .find(|k| k["id"].as_str() == Some(&id))
                    .ok_or("missing key")?;
                let c = credential(key, &resource)?;
                if c.value.is_none() {
                    return Ok(error(
                        "value_unavailable",
                        "The connection cannot reveal this key.",
                    ));
                }
                done(Outcome::Credential(c))
            }
            Operation::CreateCredential { resource, .. } => {
                let c = credential(&v, &resource)?;
                if c.value.is_none() {
                    return Ok(error(
                        "value_unavailable",
                        "The key was created but its value was not returned. Inspect the provider before retrying.",
                    ));
                }
                done(Outcome::Credential(c))
            }
            _ => error("unsupported", "Supabase does not support this operation."),
        });
    }
    let mut req = match &call.operation {
        Operation::Identity | Operation::ListOwners => {
            get("https://api.supabase.com/v1/organizations".into())
        }
        Operation::ListResources => get("https://api.supabase.com/v1/projects".into()),
        Operation::ListCredentials { resource }
        | Operation::ReadCredential { resource, .. }
        | Operation::CreateCredential { resource, .. } => get(format!(
            "https://api.supabase.com/v1/projects/{}/api-keys",
            segment(resource)
        )),
        Operation::RevokeCredential {
            resource,
            credential,
        } => get(format!(
            "https://api.supabase.com/v1/projects/{}/api-keys/{}",
            segment(resource),
            segment(credential)
        )),
        Operation::ValidateCredential { resource } => {
            if resource.len() != 20 || !resource.bytes().all(|b| b.is_ascii_lowercase()) {
                return Ok(error(
                    "invalid_resource",
                    "Select a valid Supabase project.",
                ));
            }
            let mut r = get(format!("https://{resource}.supabase.co/auth/v1/settings"));
            r.auth = RequestAuth::Candidate;
            r.headers
                .insert("x-getkeys-auth-header".into(), "apikey".into());
            r
        }
        _ => {
            return Ok(error(
                "unsupported",
                "Supabase does not support this operation.",
            ));
        }
    };
    match call.operation {
        Operation::ReadCredential { .. } | Operation::ListCredentials { .. } => {
            req.url.push_str("?reveal=true")
        }
        Operation::CreateCredential { name, .. } => {
            req.method = "POST".into();
            req.url.push_str("?reveal=true");
            req.body = Some(json!({"name": name, "type": "secret"}));
        }
        Operation::RevokeCredential { .. } => req.method = "DELETE".into(),
        _ => {}
    }
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
    fn call(operation: Operation, body: Option<Value>) -> Call {
        Call {
            operation,
            config: BTreeMap::new(),
            cursor: Value::Null,
            response: body.map(|v| HttpResponse {
                status: 200,
                body: v.to_string(),
                headers: BTreeMap::new(),
            }),
        }
    }
    #[test]
    fn listing_keeps_each_key_value() {
        let Step::Complete {
            result: Outcome::Credentials(keys),
        } = handle(call(
            Operation::ListCredentials {
                resource: "project".into(),
            },
            Some(json!([{"id":"k1","type":"secret","name":"backend","api_key":"sensitive"}])),
        ))
        else {
            panic!()
        };
        assert!(keys[0].value.is_some());
        assert!(keys[0].can_rotate);
    }
    #[test]
    fn default_keys_are_named_by_type() {
        assert_eq!(key_name("publishable", "default"), "Publishable key");
        assert_eq!(key_name("secret", "default"), "Secret key");
        assert_eq!(key_name("secret", "backend"), "backend · secret key");
        assert_eq!(key_name("legacy", "anon"), "Legacy anon key");
    }
    #[test]
    fn legacy_keys_cannot_use_modern_rotation() {
        assert!(
            !credential(
                &json!({"id":"service_role","name":"service_role","type":"legacy"}),
                "p"
            )
            .unwrap()
            .can_rotate
        );
    }
    #[test]
    fn create_requests_a_revealed_secret() {
        let Step::Request { request, .. } = handle(call(
            Operation::CreateCredential {
                resource: "abc".into(),
                name: "backend".into(),
            },
            None,
        )) else {
            panic!()
        };
        assert_eq!(request.method, "POST");
        assert!(request.url.ends_with("?reveal=true"));
        assert_eq!(request.body.unwrap()["type"], "secret");
    }
    fn new_project(response: Option<Value>) -> Step {
        handle(call(
            Operation::CreateResource {
                owner: "acme".into(),
                name: "my-app".into(),
                options: BTreeMap::from([("region".into(), "emea".into())]),
            },
            response,
        ))
    }
    #[test]
    fn new_projects_get_an_injected_database_password() {
        let Step::Request { request, .. } = new_project(None) else {
            panic!()
        };
        let body = request.body.unwrap();
        assert_eq!(body["organization_slug"], "acme");
        assert_eq!(body["region_selection"]["code"], "emea");
        assert!(body.get("db_pass").is_none());
        assert!(
            matches!(request.secret, Some(SecretInjection::JsonField { field }) if field == "db_pass")
        );
        let Step::Complete {
            result: Outcome::Resource(project),
        } = new_project(Some(
            json!({"id":"old","ref":"abcdefghijklmnopqrst","name":"my-app","status":"COMING_UP"}),
        ))
        else {
            panic!()
        };
        assert_eq!(project.id, "abcdefghijklmnopqrst");
        assert!(project.pending);
        assert!(project.url.ends_with("/project/abcdefghijklmnopqrst"));
    }
    #[test]
    fn owners_are_organizations_by_slug() {
        let Step::Complete {
            result: Outcome::Owners(owners),
        } = handle(call(
            Operation::ListOwners,
            Some(json!([{"id":"x","slug":"acme","name":"Acme"}])),
        ))
        else {
            panic!()
        };
        assert_eq!(
            (owners[0].id.as_str(), owners[0].name.as_str()),
            ("acme", "Acme")
        );
    }
}
