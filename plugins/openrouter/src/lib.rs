use getkeys_plugin_api::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub const MANIFEST: &str = include_str!("../manifest.json");
const API: &str = "https://openrouter.ai/api/v1";
/// Used when the account has no workspaces to list.
const DEFAULT: &str = "default";

pub fn handle(call: Call) -> Step {
    execute(call).unwrap_or_else(|_| {
        error(
            "invalid_response",
            "OpenRouter returned an unexpected response.",
        )
    })
}

fn keys_url(resource: &str, offset: usize) -> String {
    let mut url = format!("{API}/keys?include_disabled=true&offset={offset}");
    if resource != DEFAULT {
        url.push_str(&format!("&workspace_id={}", segment(resource)));
    }
    url
}

fn key_url(hash: &str) -> String {
    format!("{API}/keys/{}", segment(hash))
}

/// An API key. OpenRouter shows a key's value only when it is created.
fn key(k: &Value, resource: &str) -> Result<RemoteCredential, &'static str> {
    let mut details = BTreeMap::new();
    if let Some(label) = k["label"].as_str() {
        details.insert("hint".into(), label.into());
    }
    if k["disabled"] == true {
        details.insert("status".into(), "disabled".into());
    }
    if let Some(limit) = k["limit"].as_f64() {
        details.insert("limit".into(), format!("${limit}"));
    }
    if let Some(at) = k["last_used_at"].as_str() {
        details.insert("last_used".into(), at.into());
    }
    Ok(RemoteCredential {
        id: k["hash"].as_str().ok_or("hash")?.into(),
        name: k["name"].as_str().unwrap_or("OpenRouter key").into(),
        credential_type: "api_key".into(),
        resource: resource.into(),
        created_at: k["created_at"].as_str().map(Into::into),
        expires_at: k["expires_at"].as_str().map(Into::into),
        value: None,
        // A replacement starts without the old key's credit limit or expiry.
        can_rotate: k["limit"].is_null() && k["expires_at"].is_null(),
        details,
    })
}

fn execute(call: Call) -> Result<Step, &'static str> {
    if let Some(response) = &call.response {
        if let Some(error) = provider_error(response) {
            return Ok(error);
        }
        if matches!(
            call.operation,
            Operation::ValidateCredential { .. }
                | Operation::DeactivateCredential { .. }
                | Operation::RevokeCredential { .. }
        ) {
            return Ok(done(Outcome::Done));
        }
        let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
        return Ok(match &call.operation {
            Operation::Identity => {
                let d = &v["data"];
                if d["is_management_key"] != true && d["is_provisioning_key"] != true {
                    return Ok(error(
                        "not_management_key",
                        "Connect with an OpenRouter management key, not an inference key.",
                    ));
                }
                let (id, name) = match d["organization_id"].as_str() {
                    Some(org) => (org, "OpenRouter organization"),
                    None => (
                        d["creator_user_id"].as_str().ok_or("creator")?,
                        "OpenRouter account",
                    ),
                };
                done(Outcome::Identity(Identity {
                    id: id.into(),
                    name: name.into(),
                }))
            }
            Operation::ListResources => {
                let mut items = call.cursor["items"].as_array().cloned().unwrap_or_default();
                let page = v["data"].as_array().ok_or("data")?;
                items.extend(page.iter().cloned());
                let total = v["total_count"].as_u64().unwrap_or(0) as usize;
                if !page.is_empty() && items.len() < total {
                    return Ok(request(
                        get(format!("{API}/workspaces?limit=100&offset={}", items.len())),
                        json!({"items": items}),
                    ));
                }
                let mut resources = items
                    .iter()
                    .map(|w| {
                        Ok(Resource {
                            id: w["id"].as_str().ok_or("id")?.into(),
                            name: w["name"].as_str().ok_or("name")?.into(),
                            kind: "workspace".into(),
                            ..Default::default()
                        })
                    })
                    .collect::<Result<Vec<_>, &'static str>>()?;
                if resources.is_empty() {
                    resources.push(Resource {
                        id: DEFAULT.into(),
                        name: "API keys".into(),
                        kind: "workspace".into(),
                        ..Default::default()
                    });
                }
                done(Outcome::Resources(resources))
            }
            Operation::ListCredentials { resource } => {
                let mut items = call.cursor["items"].as_array().cloned().unwrap_or_default();
                let page = v["data"].as_array().ok_or("data")?;
                if !page.is_empty() {
                    items.extend(page.iter().cloned());
                    return Ok(request(
                        get(keys_url(resource, items.len())),
                        json!({"items": items}),
                    ));
                }
                done(Outcome::Credentials(
                    items
                        .iter()
                        .map(|k| key(k, resource))
                        .collect::<Result<_, _>>()?,
                ))
            }
            Operation::CreateCredential { resource, .. } => {
                let mut c = key(&v["data"], resource)?;
                let Some(value) = v["key"].as_str().filter(|s| !s.is_empty()) else {
                    return Ok(error(
                        "value_unavailable",
                        "OpenRouter created the key but did not return it. Delete it in OpenRouter before retrying.",
                    ));
                };
                c.value = Some(SecretValue {
                    fields: BTreeMap::from([("token".into(), value.into())]),
                });
                done(Outcome::Credential(c))
            }
            _ => error("unsupported", "This OpenRouter operation is unavailable."),
        });
    }
    let mut req = match &call.operation {
        Operation::Identity => get(format!("{API}/key")),
        Operation::ListResources => {
            return Ok(request(
                get(format!("{API}/workspaces?limit=100&offset=0")),
                json!({"items": []}),
            ));
        }
        Operation::ListCredentials { resource } => {
            return Ok(request(get(keys_url(resource, 0)), json!({"items": []})));
        }
        Operation::ValidateCredential { .. } => {
            let mut r = get(format!("{API}/key"));
            r.auth = RequestAuth::Candidate;
            r
        }
        Operation::CreateCredential { resource, name } => {
            let mut body = json!({"name": name});
            if resource != DEFAULT {
                body["workspace_id"] = json!(resource);
            }
            let mut r = get(format!("{API}/keys"));
            r.method = "POST".into();
            r.body = Some(body);
            r
        }
        Operation::DeactivateCredential { credential, .. } => {
            let mut r = get(key_url(credential));
            r.method = "PATCH".into();
            r.body = Some(json!({"disabled": true}));
            r
        }
        Operation::RevokeCredential { credential, .. } => get(key_url(credential)),
        _ => {
            return Ok(error(
                "unsupported",
                "This OpenRouter operation is unavailable.",
            ));
        }
    };
    if matches!(call.operation, Operation::RevokeCredential { .. }) {
        req.method = "DELETE".into();
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
    fn call(operation: Operation, cursor: Value, body: Option<Value>) -> Step {
        handle(Call {
            operation,
            config: BTreeMap::new(),
            cursor,
            response: body.map(|b| HttpResponse {
                status: 200,
                body: b.to_string(),
                headers: BTreeMap::new(),
            }),
        })
    }
    #[test]
    fn identity_requires_a_management_key() {
        assert!(matches!(
            call(
                Operation::Identity,
                Value::Null,
                Some(json!({"data":{"is_management_key":false,"creator_user_id":"user_1"}}))
            ),
            Step::Error { .. }
        ));
        let Step::Complete {
            result: Outcome::Identity(id),
        } = call(
            Operation::Identity,
            Value::Null,
            Some(
                json!({"data":{"is_management_key":true,"creator_user_id":"user_1","organization_id":null}}),
            ),
        )
        else {
            panic!()
        };
        assert_eq!(id.id, "user_1");
    }
    #[test]
    fn lists_keys_page_by_page_within_a_workspace() {
        let list = Operation::ListCredentials {
            resource: "ws_1".into(),
        };
        let Step::Request { request, cursor } = call(
            list.clone(),
            json!({"items": []}),
            Some(json!({"data":[
                {"hash":"h1","name":"app","label":"sk-or-v1-0e6...1c96","disabled":false,"limit":null,"created_at":"2025-08-24T10:30:00Z","expires_at":null},
                {"hash":"h2","name":"capped","label":"sk-or-v1-1a2...3b4c","disabled":true,"limit":50,"created_at":"2025-08-24T10:30:00Z","expires_at":null}
            ]})),
        ) else {
            panic!()
        };
        assert!(request.url.contains("offset=2&workspace_id=ws_1"));
        let Step::Complete {
            result: Outcome::Credentials(keys),
        } = call(list, cursor, Some(json!({"data":[]})))
        else {
            panic!()
        };
        assert_eq!(keys[0].id, "h1");
        assert!(keys[0].can_rotate && !keys[1].can_rotate);
        assert_eq!(keys[1].details["status"], "disabled");
        assert_eq!(keys[1].details["limit"], "$50");
    }
    #[test]
    fn accounts_without_workspaces_get_one_default_resource() {
        let Step::Complete {
            result: Outcome::Resources(r),
        } = call(
            Operation::ListResources,
            json!({"items": []}),
            Some(json!({"data":[],"total_count":0})),
        )
        else {
            panic!()
        };
        assert_eq!(r[0].id, DEFAULT);
        let Step::Request { request, .. } = call(
            Operation::ListCredentials {
                resource: DEFAULT.into(),
            },
            Value::Null,
            None,
        ) else {
            panic!()
        };
        assert!(!request.url.contains("workspace_id"));
    }
    #[test]
    fn created_keys_carry_their_one_time_value() {
        let op = Operation::CreateCredential {
            resource: "ws_1".into(),
            name: "GetKeys".into(),
        };
        let Step::Request { request, .. } = call(op.clone(), Value::Null, None) else {
            panic!()
        };
        assert_eq!(request.body.unwrap()["workspace_id"], "ws_1");
        let Step::Complete {
            result: Outcome::Credential(c),
        } = call(
            op,
            Value::Null,
            Some(
                json!({"data":{"hash":"h3","name":"GetKeys","limit":null,"expires_at":null},"key":"sk-or-v1-test"}),
            ),
        )
        else {
            panic!()
        };
        assert_eq!(c.id, "h3");
        assert!(c.value.is_some());
    }
    #[test]
    fn deactivating_disables_and_revoking_deletes() {
        let Step::Request { request, .. } = call(
            Operation::DeactivateCredential {
                resource: "ws_1".into(),
                credential: "h1".into(),
            },
            Value::Null,
            None,
        ) else {
            panic!()
        };
        assert_eq!(request.method, "PATCH");
        let Step::Request { request, .. } = call(
            Operation::RevokeCredential {
                resource: "ws_1".into(),
                credential: "h1".into(),
            },
            Value::Null,
            None,
        ) else {
            panic!()
        };
        assert_eq!(request.method, "DELETE");
        assert!(request.url.ends_with("/keys/h1"));
    }
}
