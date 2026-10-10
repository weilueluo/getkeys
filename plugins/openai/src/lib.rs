use getkeys_plugin_api::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub const MANIFEST: &str = include_str!("../manifest.json");
const API: &str = "https://api.openai.com/v1";
/// Pseudo-resource holding the organization's admin keys.
const ADMIN: &str = "admin";

pub fn handle(call: Call) -> Step {
    execute(call).unwrap_or_else(|_| {
        error(
            "invalid_response",
            "OpenAI returned an unexpected response.",
        )
    })
}

/// RFC 3339 time of a Unix timestamp in seconds.
fn iso(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn project(id: &str) -> String {
    format!("{API}/organization/projects/{}", segment(id))
}

/// The paginated list an operation reads.
fn list_path(operation: &Operation) -> Option<String> {
    match operation {
        Operation::Identity => Some(format!("{API}/organization/projects?include_archived=true")),
        Operation::ListResources => Some(format!("{API}/organization/projects?")),
        Operation::ListCredentials { resource } if resource == ADMIN => {
            Some(format!("{API}/organization/admin_api_keys?"))
        }
        Operation::ListCredentials { resource } => Some(format!("{}/api_keys?", project(resource))),
        _ => None,
    }
}

fn page(path: &str, after: Option<&str>) -> HttpRequest {
    let sep = if path.ends_with('?') { "" } else { "&" };
    let mut url = format!("{path}{sep}limit=100");
    if let Some(after) = after {
        url.push_str(&format!("&after={}", segment(after)));
    }
    get(url)
}

fn send(mut req: HttpRequest, method: &str, body: Option<Value>) -> HttpRequest {
    req.method = method.into();
    req.body = body;
    req
}

/// A project or admin key. OpenAI shows a key's value only when it is created.
fn key(k: &Value, resource: &str) -> Result<RemoteCredential, &'static str> {
    let owner = k["owner"]["type"].as_str().unwrap_or("user");
    let service = owner == "service_account";
    let mut details = BTreeMap::new();
    if let Some(hint) = k["redacted_value"].as_str() {
        details.insert("hint".into(), hint.into());
    }
    if let Some(at) = k["last_used_at"].as_i64() {
        details.insert("last_used".into(), iso(at));
    }
    let who = &k["owner"][owner];
    if let Some(name) = who["name"].as_str().or(k["owner"]["name"].as_str()) {
        details.insert("owner".into(), name.into());
    }
    Ok(RemoteCredential {
        id: k["id"].as_str().ok_or("id")?.into(),
        name: k["name"].as_str().unwrap_or("OpenAI key").into(),
        credential_type: if resource == ADMIN {
            "admin_key"
        } else if service {
            "service_account"
        } else {
            "api_key"
        }
        .into(),
        resource: resource.into(),
        created_at: k["created_at"].as_i64().map(iso),
        expires_at: None,
        value: None,
        // GetKeys can replace keys it can create: service account keys and admin keys.
        can_rotate: service || resource == ADMIN,
        details,
    })
}

fn execute(call: Call) -> Result<Step, &'static str> {
    if let Some(response) = &call.response {
        if let Some(error) = provider_error(response) {
            return Ok(error);
        }
        match &call.operation {
            Operation::ValidateCredential { .. } => return Ok(done(Outcome::Done)),
            Operation::RevokeCredential { .. } if call.cursor["stage"] != "lookup" => {
                return Ok(done(Outcome::Done));
            }
            _ => {}
        }
        let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
        if let Some(path) = list_path(&call.operation) {
            let mut items = call.cursor["items"].as_array().cloned().unwrap_or_default();
            items.extend(v["data"].as_array().ok_or("data")?.iter().cloned());
            if v["has_more"] == true {
                let after = v["last_id"].as_str().ok_or("last_id")?;
                return Ok(request(page(&path, Some(after)), json!({"items": items})));
            }
            return Ok(match &call.operation {
                Operation::Identity => {
                    // Admin keys can't read the organization itself; its first project is permanent.
                    let first = items
                        .iter()
                        .min_by_key(|p| p["created_at"].as_i64().unwrap_or(i64::MAX))
                        .ok_or("no projects")?;
                    done(Outcome::Identity(Identity {
                        id: first["id"].as_str().ok_or("id")?.into(),
                        name: "OpenAI organization".into(),
                    }))
                }
                Operation::ListResources => {
                    let mut resources = items
                        .iter()
                        .map(|p| {
                            Ok(Resource {
                                id: p["id"].as_str().ok_or("id")?.into(),
                                name: p["name"].as_str().ok_or("name")?.into(),
                                kind: "project".into(),
                            })
                        })
                        .collect::<Result<Vec<_>, &'static str>>()?;
                    resources.push(Resource {
                        id: ADMIN.into(),
                        name: "Admin keys".into(),
                        kind: "admin_keys".into(),
                    });
                    done(Outcome::Resources(resources))
                }
                Operation::ListCredentials { resource } => done(Outcome::Credentials(
                    items
                        .iter()
                        .map(|k| key(k, resource))
                        .collect::<Result<_, _>>()?,
                )),
                _ => unreachable!(),
            });
        }
        return Ok(match &call.operation {
            Operation::CreateCredential { resource, .. } => {
                let created = if resource == ADMIN {
                    v.clone()
                } else {
                    let mut k = v["api_key"].clone();
                    k["owner"] = json!({"type": "service_account", "service_account": {"id": v["id"], "name": v["name"]}});
                    k
                };
                let mut c = key(&created, resource)?;
                let Some(value) = created["value"].as_str().filter(|s| !s.is_empty()) else {
                    return Ok(error(
                        "value_unavailable",
                        "OpenAI created the key but did not return it. Delete it in OpenAI before retrying.",
                    ));
                };
                c.value = Some(SecretValue {
                    fields: BTreeMap::from([("token".into(), value.into())]),
                });
                done(Outcome::Credential(c))
            }
            Operation::RevokeCredential {
                resource,
                credential,
            } => {
                // A service account's key can only go with its service account.
                let url = match v["owner"]["service_account"]["id"].as_str() {
                    Some(sa) if v["owner"]["type"] == "service_account" => {
                        format!("{}/service_accounts/{}", project(resource), segment(sa))
                    }
                    _ => format!("{}/api_keys/{}", project(resource), segment(credential)),
                };
                request(send(get(url), "DELETE", None), json!({"stage": "delete"}))
            }
            _ => error("unsupported", "This OpenAI operation is unavailable."),
        });
    }
    if let Some(path) = list_path(&call.operation) {
        return Ok(request(page(&path, None), json!({"items": []})));
    }
    let req = match &call.operation {
        Operation::ValidateCredential { resource } => {
            let mut r = if resource.is_empty() || resource == ADMIN {
                get(format!("{API}/organization/projects?limit=1"))
            } else {
                get(format!("{API}/models"))
            };
            r.auth = RequestAuth::Candidate;
            r
        }
        Operation::CreateCredential { resource, name } if resource == ADMIN => send(
            get(format!("{API}/organization/admin_api_keys")),
            "POST",
            Some(json!({"name": name})),
        ),
        Operation::CreateCredential { resource, name } if !resource.is_empty() => send(
            get(format!("{}/service_accounts", project(resource))),
            "POST",
            Some(json!({"name": name})),
        ),
        Operation::RevokeCredential {
            resource,
            credential,
        } if resource == ADMIN => send(
            get(format!(
                "{API}/organization/admin_api_keys/{}",
                segment(credential)
            )),
            "DELETE",
            None,
        ),
        Operation::RevokeCredential {
            resource,
            credential,
        } if !resource.is_empty() => {
            return Ok(request(
                get(format!(
                    "{}/api_keys/{}",
                    project(resource),
                    segment(credential)
                )),
                json!({"stage": "lookup"}),
            ));
        }
        Operation::Guide { .. } => {
            return Ok(done(Outcome::Guide {
                url: "https://platform.openai.com/api-keys".into(),
                message: "Keys owned by people can't be created through the API. GetKeys creates service account keys instead; replace a personal key with one, update consumers, then delete the old key in OpenAI.".into(),
            }));
        }
        _ => {
            return Ok(error(
                "unsupported",
                "This OpenAI operation is unavailable.",
            ));
        }
    };
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
    fn identity_is_the_oldest_project_across_pages() {
        let Step::Request { request, cursor } = call(
            Operation::Identity,
            json!({"items": []}),
            Some(
                json!({"data":[{"id":"proj_b","created_at":20}],"has_more":true,"last_id":"proj_b"}),
            ),
        ) else {
            panic!()
        };
        assert!(
            request
                .url
                .ends_with("include_archived=true&limit=100&after=proj_b")
        );
        let Step::Complete {
            result: Outcome::Identity(id),
        } = call(
            Operation::Identity,
            cursor,
            Some(json!({"data":[{"id":"proj_a","created_at":10}],"has_more":false})),
        )
        else {
            panic!()
        };
        assert_eq!(id.id, "proj_a");
    }
    #[test]
    fn resources_include_admin_keys() {
        let Step::Complete {
            result: Outcome::Resources(r),
        } = call(
            Operation::ListResources,
            json!({"items": []}),
            Some(json!({"data":[{"id":"proj_1","name":"Default project"}],"has_more":false})),
        )
        else {
            panic!()
        };
        assert_eq!(r[0].kind, "project");
        assert_eq!(r[1].id, ADMIN);
    }
    #[test]
    fn project_keys_are_secretless_and_service_accounts_rotate() {
        let Step::Complete {
            result: Outcome::Credentials(keys),
        } = call(
            Operation::ListCredentials {
                resource: "proj_1".into(),
            },
            json!({"items": []}),
            Some(json!({"data":[
                {"id":"key_1","name":"mine","redacted_value":"sk-abc...def","created_at":0,"last_used_at":null,"owner":{"type":"user","user":{"name":"Ada"}}},
                {"id":"key_2","name":"ci","redacted_value":"sk-svc...xyz","created_at":1_700_000_000,"owner":{"type":"service_account","service_account":{"id":"svc_1","name":"ci"}}}
            ],"has_more":false})),
        )
        else {
            panic!()
        };
        assert_eq!(keys[0].credential_type, "api_key");
        assert_eq!(keys[0].details["owner"], "Ada");
        assert!(!keys[0].can_rotate && keys[1].can_rotate);
        assert_eq!(keys[1].created_at.as_deref(), Some("2023-11-14T22:13:20Z"));
        assert!(keys.iter().all(|k| k.value.is_none()));
    }
    #[test]
    fn creates_project_keys_as_service_accounts() {
        let op = Operation::CreateCredential {
            resource: "proj_1".into(),
            name: "GetKeys".into(),
        };
        let Step::Request { request, .. } = call(op.clone(), Value::Null, None) else {
            panic!()
        };
        assert_eq!(request.method, "POST");
        assert!(request.url.ends_with("/projects/proj_1/service_accounts"));
        let Step::Complete {
            result: Outcome::Credential(c),
        } = call(
            op,
            Value::Null,
            Some(
                json!({"id":"svc_1","name":"GetKeys","api_key":{"id":"key_9","name":"Secret Key","value":"sk-test","created_at":1}}),
            ),
        )
        else {
            panic!()
        };
        assert_eq!(c.id, "key_9");
        assert!(c.can_rotate && c.value.is_some());
    }
    #[test]
    fn revoking_a_service_account_key_deletes_the_service_account() {
        let op = Operation::RevokeCredential {
            resource: "proj_1".into(),
            credential: "key_2".into(),
        };
        let Step::Request { request, cursor } = call(op.clone(), Value::Null, None) else {
            panic!()
        };
        assert_eq!(request.method, "GET");
        let Step::Request { request, cursor } = call(
            op.clone(),
            cursor,
            Some(
                json!({"id":"key_2","owner":{"type":"service_account","service_account":{"id":"svc_1"}}}),
            ),
        ) else {
            panic!()
        };
        assert_eq!(request.method, "DELETE");
        assert!(
            request
                .url
                .ends_with("/projects/proj_1/service_accounts/svc_1")
        );
        assert!(matches!(
            call(op, cursor, Some(json!({"deleted": true}))),
            Step::Complete {
                result: Outcome::Done
            }
        ));
    }
    #[test]
    fn validation_uses_the_candidate_key() {
        let Step::Request { request, .. } = call(
            Operation::ValidateCredential {
                resource: "proj_1".into(),
            },
            Value::Null,
            None,
        ) else {
            panic!()
        };
        assert!(matches!(request.auth, RequestAuth::Candidate));
        assert!(request.url.ends_with("/v1/models"));
    }
}
