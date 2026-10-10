use getkeys_plugin_api::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub const MANIFEST: &str = include_str!("../manifest.json");
const API: &str = "https://gitlab.com/api/v4";
const PER_PAGE: usize = 100;

pub fn handle(call: Call) -> Step {
    execute(call).unwrap_or_else(|_| {
        error(
            "invalid_response",
            "GitLab returned an unexpected response.",
        )
    })
}

fn token_url(id: &str) -> String {
    format!("{API}/personal_access_tokens/{}", segment(id))
}

fn page(user: &Value, n: u64) -> HttpRequest {
    get(format!(
        "{API}/personal_access_tokens?user_id={user}&state=active&per_page={PER_PAGE}&page={n}"
    ))
}

/// A personal access token. GitLab shows a token's value only when it is created or rotated.
fn token(t: &Value, current: &Value) -> Result<RemoteCredential, &'static str> {
    let id = t["id"].as_i64().ok_or("token id")?;
    let mut details = BTreeMap::new();
    if let Some(scopes) = t["scopes"].as_array() {
        let scopes: Vec<&str> = scopes.iter().filter_map(Value::as_str).collect();
        details.insert("scopes".into(), scopes.join(", "));
    }
    if let Some(at) = t["last_used_at"].as_str() {
        details.insert("last_used".into(), at.into());
    }
    Ok(RemoteCredential {
        id: id.to_string(),
        name: t["name"].as_str().unwrap_or("GitLab token").into(),
        credential_type: if t["id"] == *current {
            "sign_in"
        } else {
            "personal_access_token"
        }
        .into(),
        resource: String::new(),
        created_at: t["created_at"].as_str().map(Into::into),
        expires_at: t["expires_at"].as_str().map(|d| format!("{d}T00:00:00Z")),
        value: None,
        can_rotate: false,
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
            Operation::ValidateCredential { .. } | Operation::RevokeCredential { .. }
        ) {
            return Ok(done(Outcome::Done));
        }
        let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
        return Ok(match (&call.operation, call.cursor["stage"].as_str()) {
            (Operation::Identity, _) => done(Outcome::Identity(Identity {
                id: v["id"].as_i64().ok_or("id")?.to_string(),
                name: v["username"].as_str().ok_or("username")?.into(),
            })),
            (Operation::ListCredentials { .. }, Some("self")) => request(
                page(&v["user_id"], 1),
                json!({"current": v["id"], "user": v["user_id"], "items": [], "page": 1}),
            ),
            (Operation::ListCredentials { .. }, _) => {
                let mut items = call.cursor["items"].as_array().cloned().unwrap_or_default();
                let batch = v.as_array().ok_or("tokens")?;
                items.extend(batch.iter().cloned());
                if batch.len() == PER_PAGE {
                    let n = call.cursor["page"].as_u64().unwrap_or(1) + 1;
                    let mut cursor = call.cursor.clone();
                    cursor["items"] = json!(items);
                    cursor["page"] = json!(n);
                    return Ok(request(page(&call.cursor["user"], n), cursor));
                }
                done(Outcome::Credentials(
                    items
                        .iter()
                        .map(|t| token(t, &call.cursor["current"]))
                        .collect::<Result<_, _>>()?,
                ))
            }
            (Operation::RotateCredential { credential, .. }, Some("self")) => {
                if v["id"].as_i64().map(|id| id.to_string()).as_ref() == Some(credential) {
                    return Ok(error(
                        "in_use",
                        "This token signs GetKeys in to GitLab. Reconnect with another token to rotate it.",
                    ));
                }
                request(get(token_url(credential)), json!({"stage": "read"}))
            }
            (Operation::RotateCredential { credential, .. }, Some("read")) => {
                let mut req = get(format!("{}/rotate", token_url(credential)));
                req.method = "POST".into();
                // Without an expiry GitLab gives the new token one week.
                if let Some(expires) = v["expires_at"].as_str() {
                    req.body = Some(json!({"expires_at": expires}));
                }
                request(req, json!({"stage": "rotate"}))
            }
            (Operation::RotateCredential { .. }, _) => {
                let mut c = token(&v, &Value::Null)?;
                let Some(value) = v["token"].as_str().filter(|s| !s.is_empty()) else {
                    return Ok(error(
                        "value_unavailable",
                        "GitLab rotated the token but did not return the new value. Rotate it again in GitLab.",
                    ));
                };
                c.value = Some(SecretValue {
                    fields: BTreeMap::from([("token".into(), value.into())]),
                });
                done(Outcome::Credential(c))
            }
            _ => error("unsupported", "This GitLab operation is unavailable."),
        });
    }
    let (req, cursor) = match &call.operation {
        Operation::Identity => (get(format!("{API}/user")), Value::Null),
        Operation::ListCredentials { resource } if resource.is_empty() => (
            get(format!("{API}/personal_access_tokens/self")),
            json!({"stage": "self"}),
        ),
        Operation::ListCredentials { .. } => return Ok(done(Outcome::Credentials(vec![]))),
        Operation::ValidateCredential { .. } => {
            let mut r = get(format!("{API}/personal_access_tokens/self"));
            r.auth = RequestAuth::Candidate;
            (r, Value::Null)
        }
        Operation::RotateCredential { .. } => (
            get(format!("{API}/personal_access_tokens/self")),
            json!({"stage": "self"}),
        ),
        Operation::RevokeCredential { credential, .. } => {
            let mut r = get(token_url(credential));
            r.method = "DELETE".into();
            (r, Value::Null)
        }
        _ => {
            return Ok(error(
                "unsupported",
                "This GitLab operation is unavailable.",
            ));
        }
    };
    Ok(request(req, cursor))
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
    fn rotate() -> Operation {
        Operation::RotateCredential {
            resource: String::new(),
            credential: "42".into(),
        }
    }
    #[test]
    fn lists_the_users_active_tokens_and_marks_the_one_in_use() {
        let list = Operation::ListCredentials {
            resource: String::new(),
        };
        let Step::Request { request, cursor } = call(
            list.clone(),
            json!({"stage": "self"}),
            Some(json!({"id": 7, "user_id": 3})),
        ) else {
            panic!()
        };
        assert!(request.url.contains("user_id=3&state=active"));
        let Step::Complete {
            result: Outcome::Credentials(keys),
        } = call(
            list,
            cursor,
            Some(json!([
                {"id": 7, "name": "getkeys", "scopes": ["api"], "created_at": "2024-01-01T00:00:00.000Z", "expires_at": "2025-01-01"},
                {"id": 42, "name": "ci", "scopes": ["read_api", "read_repository"], "expires_at": null}
            ])),
        )
        else {
            panic!()
        };
        assert_eq!(keys[0].credential_type, "sign_in");
        assert_eq!(keys[0].expires_at.as_deref(), Some("2025-01-01T00:00:00Z"));
        assert_eq!(keys[1].details["scopes"], "read_api, read_repository");
    }
    #[test]
    fn rotation_keeps_the_expiry_and_returns_the_new_token() {
        let Step::Request { cursor, .. } =
            call(rotate(), json!({"stage": "self"}), Some(json!({"id": 7})))
        else {
            panic!()
        };
        let Step::Request { request, cursor } = call(
            rotate(),
            cursor,
            Some(json!({"id": 42, "expires_at": "2026-05-01"})),
        ) else {
            panic!()
        };
        assert_eq!(request.method, "POST");
        assert!(request.url.ends_with("/personal_access_tokens/42/rotate"));
        assert_eq!(request.body.unwrap()["expires_at"], "2026-05-01");
        let Step::Complete {
            result: Outcome::Credential(c),
        } = call(
            rotate(),
            cursor,
            Some(json!({"id": 43, "name": "ci", "token": "glpat-new", "expires_at": "2026-05-01"})),
        )
        else {
            panic!()
        };
        assert_eq!(c.id, "43");
        assert!(c.value.is_some());
    }
    #[test]
    fn refuses_to_rotate_the_token_in_use() {
        assert!(matches!(
            call(rotate(), json!({"stage": "self"}), Some(json!({"id": 42}))),
            Step::Error { .. }
        ));
    }
}
