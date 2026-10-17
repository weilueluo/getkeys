use getkeys_plugin_api::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const MANIFEST: &str = include_str!("../manifest.json");
const API: &str = "https://console.neon.tech/api/v2";

pub fn handle(call: Call) -> Step {
    execute(call)
        .unwrap_or_else(|_| error("invalid_response", "Neon returned an unexpected response."))
}

fn keys_url(resource: &str) -> String {
    format!("{API}/organizations/{}/api_keys", segment(resource))
}

fn key(value: &Value, resource: &str) -> Result<RemoteCredential, &'static str> {
    let mut details = BTreeMap::new();
    if let Some(at) = value["last_used_at"].as_str() {
        details.insert("last_used".into(), at.into());
    }
    if let Some(project) = value["project_id"].as_str() {
        details.insert("project".into(), project.into());
    }
    Ok(RemoteCredential {
        id: value["id"].as_i64().ok_or("id")?.to_string(),
        name: value["name"].as_str().ok_or("name")?.into(),
        credential_type: "api_key".into(),
        resource: resource.into(),
        created_at: value["created_at"].as_str().map(Into::into),
        expires_at: None,
        value: None,
        // Creating a replacement without project_id would broaden a project-scoped key.
        can_rotate: value["project_id"].is_null(),
        details,
    })
}

fn execute(call: Call) -> Result<Step, &'static str> {
    if let Some(response) = &call.response {
        if let Some(error) = provider_error(response) {
            return Ok(error);
        }
        if matches!(call.operation, Operation::ValidateCredential { .. }) {
            return Ok(done(Outcome::Done));
        }
        let value: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
        return Ok(match &call.operation {
            Operation::Identity if call.cursor == "auth" => {
                if value["auth_method"] != "api_key_user" {
                    return Ok(error(
                        "personal_key_required",
                        "Connect with a personal Neon API key belonging to an organization admin.",
                    ));
                }
                request(get(format!("{API}/users/me")), Value::Null)
            }
            Operation::Identity => done(Outcome::Identity(Identity {
                id: value["id"].as_str().ok_or("id")?.into(),
                name: value["email"].as_str().ok_or("email")?.into(),
            })),
            Operation::ListResources => done(Outcome::Resources(
                value["organizations"]
                    .as_array()
                    .ok_or("organizations")?
                    .iter()
                    .map(|org| {
                        Ok(Resource {
                            id: org["id"].as_str().ok_or("id")?.into(),
                            name: org["name"].as_str().ok_or("name")?.into(),
                            kind: "organization".into(),
                            ..Default::default()
                        })
                    })
                    .collect::<Result<_, &'static str>>()?,
            )),
            Operation::ListCredentials { resource } => done(Outcome::Credentials(
                value
                    .as_array()
                    .ok_or("keys")?
                    .iter()
                    .map(|v| key(v, resource))
                    .collect::<Result<_, _>>()?,
            )),
            Operation::CreateCredential { resource, .. } => {
                let Some(secret) = value["key"].as_str().filter(|s| !s.is_empty()) else {
                    return Ok(error(
                        "value_unavailable",
                        "Neon created the key but did not return its value. Revoke it in Neon before retrying.",
                    ));
                };
                let mut credential = key(&value, resource)?;
                credential.value = Some(SecretValue {
                    fields: BTreeMap::from([("token".into(), secret.into())]),
                });
                done(Outcome::Credential(credential))
            }
            Operation::RevokeCredential { credential, .. } => {
                if value["revoked"] != true
                    || value["id"].as_i64().map(|id| id.to_string()).as_ref() != Some(credential)
                {
                    return Err("revocation");
                }
                done(Outcome::Done)
            }
            _ => error("unsupported", "This Neon operation is unavailable."),
        });
    }
    let req = match &call.operation {
        Operation::Identity => {
            return Ok(request(get(format!("{API}/auth")), json!("auth")));
        }
        Operation::ListResources => get(format!("{API}/users/me/organizations")),
        Operation::ListCredentials { resource } => get(keys_url(resource)),
        Operation::CreateCredential { resource, name } => {
            if name.trim().is_empty() || name.chars().count() > 64 {
                return Ok(error(
                    "invalid_name",
                    "Give the Neon key a name of 1 to 64 characters.",
                ));
            }
            post(keys_url(resource), json!({"key_name": name}))
        }
        Operation::ValidateCredential { .. } => {
            let mut req = get(format!("{API}/auth"));
            req.auth = RequestAuth::Candidate;
            req
        }
        Operation::RevokeCredential {
            resource,
            credential,
        } => {
            let mut req = get(format!("{}/{}", keys_url(resource), segment(credential)));
            req.method = "DELETE".into();
            req
        }
        _ => return Ok(error("unsupported", "This Neon operation is unavailable.")),
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
            response: body.map(|body| HttpResponse {
                status: 200,
                body: body.to_string(),
                headers: BTreeMap::new(),
            }),
        })
    }

    fn create(name: &str) -> Operation {
        Operation::CreateCredential {
            resource: "org-example-123".into(),
            name: name.into(),
        }
    }

    #[test]
    fn only_personal_api_keys_can_connect_to_manage_organization_keys() {
        let Step::Request { request, cursor } = call(Operation::Identity, Value::Null, None) else {
            panic!()
        };
        assert_eq!(request.url, format!("{API}/auth"));
        for method in ["api_key_org", "oauth", "session_cookie"] {
            assert!(
                matches!(call(Operation::Identity, cursor.clone(), Some(json!({"auth_method": method}))), Step::Error { code, .. } if code == "personal_key_required")
            );
        }
        let Step::Request { request, cursor } = call(
            Operation::Identity,
            cursor,
            Some(json!({"auth_method": "api_key_user", "account_id": "user-1"})),
        ) else {
            panic!()
        };
        assert_eq!(request.url, format!("{API}/users/me"));
        let Step::Complete {
            result: Outcome::Identity(identity),
        } = call(
            Operation::Identity,
            cursor,
            Some(json!({"id": "user-1", "email": "user@example.com"})),
        )
        else {
            panic!()
        };
        assert_eq!(identity.id, "user-1");
        assert_eq!(identity.name, "user@example.com");
    }

    #[test]
    fn lists_organizations_and_keys_without_inventing_secret_values() {
        let Step::Request { request, .. } = call(Operation::ListResources, Value::Null, None)
        else {
            panic!()
        };
        assert_eq!(request.url, format!("{API}/users/me/organizations"));
        let Step::Complete {
            result: Outcome::Resources(resources),
        } = call(
            Operation::ListResources,
            Value::Null,
            Some(json!({"organizations": [{"id": "org-example-123", "name": "Example"}]})),
        )
        else {
            panic!()
        };
        assert_eq!(resources[0].kind, "organization");
        let list = Operation::ListCredentials {
            resource: resources[0].id.clone(),
        };
        let Step::Request { request, .. } = call(list.clone(), Value::Null, None) else {
            panic!()
        };
        assert_eq!(
            request.url,
            format!("{API}/organizations/org-example-123/api_keys")
        );
        let Step::Complete {
            result: Outcome::Credentials(keys),
        } = call(
            list,
            Value::Null,
            Some(json!([
                {"id": 123, "name": "Backend", "created_at": "2026-10-01T10:00:00Z", "last_used_at": "2026-10-16T10:00:00Z"},
                {"id": 124, "name": "Project", "project_id": "green-breeze-123"}
            ])),
        )
        else {
            panic!()
        };
        assert_eq!(keys[0].id, "123");
        assert_eq!(keys[0].resource, "org-example-123");
        assert!(keys[0].value.is_none());
        assert!(keys[0].can_rotate);
        assert!(!keys[1].can_rotate);
        assert_eq!(keys[1].details["project"], "green-breeze-123");
    }

    #[test]
    fn creates_keys_and_requires_the_one_time_secret() {
        let Step::Request { request, .. } = call(create("Backend"), Value::Null, None) else {
            panic!()
        };
        assert_eq!(request.method, "POST");
        assert_eq!(request.url, keys_url("org-example-123"));
        assert_eq!(request.body, Some(json!({"key_name": "Backend"})));
        let mut body = json!({"id": 125, "name": "Backend", "key": "napi_example", "created_at": "2026-10-17T10:00:00Z"});
        let Step::Complete {
            result: Outcome::Credential(key),
        } = call(create("Backend"), Value::Null, Some(body.clone()))
        else {
            panic!()
        };
        assert_eq!(key.value.unwrap().fields["token"], "napi_example");
        assert!(key.can_rotate);
        for missing in [Value::Null, json!("")] {
            body["key"] = missing;
            assert!(
                matches!(call(create("Backend"), Value::Null, Some(body.clone())), Step::Error { code, .. } if code == "value_unavailable")
            );
        }
        for name in [" ".into(), "x".repeat(65)] {
            assert!(
                matches!(call(create(&name), Value::Null, None), Step::Error { code, .. } if code == "invalid_name")
            );
        }
    }

    #[test]
    fn validates_the_candidate_and_requires_confirmed_revocation() {
        let Step::Request { request, .. } = call(
            Operation::ValidateCredential {
                resource: "org-example-123".into(),
            },
            Value::Null,
            None,
        ) else {
            panic!()
        };
        assert!(matches!(request.auth, RequestAuth::Candidate));
        assert_eq!(request.url, format!("{API}/auth"));
        let revoke = Operation::RevokeCredential {
            resource: "org-example-123".into(),
            credential: "125".into(),
        };
        let Step::Request { request, .. } = call(revoke.clone(), Value::Null, None) else {
            panic!()
        };
        assert_eq!(request.method, "DELETE");
        assert_eq!(
            request.url,
            format!("{API}/organizations/org-example-123/api_keys/125")
        );
        assert!(matches!(
            call(
                revoke.clone(),
                Value::Null,
                Some(json!({"id":125,"revoked":true}))
            ),
            Step::Complete {
                result: Outcome::Done
            }
        ));
        for body in [
            json!({"id":125,"revoked":false}),
            json!({"id":126,"revoked":true}),
        ] {
            assert!(
                matches!(call(revoke.clone(), Value::Null, Some(body)), Step::Error { code, .. } if code == "invalid_response")
            );
        }
    }

    #[test]
    fn malformed_and_failed_responses_never_become_success() {
        for body in [json!({}), json!({"organizations": [{"name":"Missing ID"}]})] {
            assert!(
                matches!(call(Operation::ListResources, Value::Null, Some(body)), Step::Error { code, .. } if code == "invalid_response")
            );
        }
        for status in [401, 403, 429, 500] {
            assert!(matches!(handle(Call {
                operation: create("Backend"), config: BTreeMap::new(), cursor: Value::Null,
                response: Some(HttpResponse { status, body: "private provider response".into(), headers: BTreeMap::new() })
            }), Step::Error { message, .. } if !message.contains("private")));
        }
        assert!(
            matches!(call(Operation::ListResources, Value::Null, Some(json!({"organizations":[]}))), Step::Complete { result: Outcome::Resources(r) } if r.is_empty())
        );
    }
}
