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
        name: v["name"].as_str().unwrap_or(kind).into(),
        credential_type: kind.into(),
        resource: resource.into(),
        created_at: v["inserted_at"].as_str().map(str::to_owned),
        expires_at: None,
        value,
        can_rotate: kind == "secret",
    })
}

fn execute(call: Call) -> Result<Step, &'static str> {
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
                    .map(|p| {
                        Ok(Resource {
                            id: p["id"].as_str().ok_or("project id")?.into(),
                            name: p["name"].as_str().ok_or("project name")?.into(),
                            kind: "project".into(),
                        })
                    })
                    .collect::<Result<_, &'static str>>()?,
            )),
            Operation::ListCredentials { resource } => {
                let keys = v
                    .as_array()
                    .ok_or("keys")?
                    .iter()
                    .map(|k| {
                        let mut c = credential(k, &resource)?;
                        c.value = None;
                        Ok(c)
                    })
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
        Operation::Identity => get("https://api.supabase.com/v1/organizations".into()),
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
        Operation::ReadCredential { .. } => req.url.push_str("?reveal=true"),
        Operation::ListCredentials { .. } => req.url.push_str("?reveal=false"),
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
    fn inventory_never_returns_values() {
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
        assert!(keys[0].value.is_none());
        assert!(keys[0].can_rotate);
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
}
