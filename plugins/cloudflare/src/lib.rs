use getkeys_plugin_api::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub const MANIFEST: &str = include_str!("../manifest.json");
const API: &str = "https://api.cloudflare.com/client/v4";

pub fn handle(call: Call) -> Step {
    execute(call).unwrap_or_else(|_| {
        error(
            "invalid_response",
            "Cloudflare returned an unexpected response.",
        )
    })
}

fn token_url(id: &str) -> String {
    format!("{API}/user/tokens/{}", segment(id))
}

fn page(n: u64) -> HttpRequest {
    get(format!("{API}/user/tokens?per_page=50&page={n}"))
}

/// A user API token. Cloudflare shows a token's value only when it is created or rolled.
fn token(t: &Value, current: &str) -> Result<RemoteCredential, &'static str> {
    let id = t["id"].as_str().ok_or("token id")?;
    let mut details = BTreeMap::new();
    if let Some(status) = t["status"].as_str() {
        details.insert("status".into(), status.into());
    }
    if let Some(at) = t["last_used_on"].as_str() {
        details.insert("last_used".into(), at.into());
    }
    Ok(RemoteCredential {
        id: id.into(),
        name: t["name"].as_str().unwrap_or("Cloudflare token").into(),
        credential_type: if id == current {
            "sign_in"
        } else {
            "api_token"
        }
        .into(),
        resource: String::new(),
        created_at: t["issued_on"].as_str().map(Into::into),
        expires_at: t["expires_on"].as_str().map(Into::into),
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
        if v["success"] == false {
            return Ok(error(
                "provider_error",
                "Cloudflare did not accept the request.",
            ));
        }
        let r = &v["result"];
        return Ok(match (&call.operation, call.cursor["stage"].as_str()) {
            (Operation::Identity, _) => done(Outcome::Identity(Identity {
                id: r["id"].as_str().ok_or("id")?.into(),
                name: r["email"].as_str().ok_or("email")?.into(),
            })),
            (Operation::ListCredentials { .. }, Some("current")) => {
                request(page(1), json!({"current": r["id"], "items": [], "page": 1}))
            }
            (Operation::ListCredentials { .. }, _) => {
                let mut items = call.cursor["items"].as_array().cloned().unwrap_or_default();
                let batch = r.as_array().ok_or("result")?;
                items.extend(batch.iter().cloned());
                let total = v["result_info"]["total_count"].as_u64().unwrap_or(0) as usize;
                if !batch.is_empty() && items.len() < total {
                    let n = call.cursor["page"].as_u64().unwrap_or(1) + 1;
                    return Ok(request(
                        page(n),
                        json!({"current": call.cursor["current"], "items": items, "page": n}),
                    ));
                }
                let current = call.cursor["current"].as_str().unwrap_or_default();
                done(Outcome::Credentials(
                    items
                        .iter()
                        .map(|t| token(t, current))
                        .collect::<Result<_, _>>()?,
                ))
            }
            (Operation::RotateCredential { credential, .. }, Some("verify")) => {
                if r["id"].as_str() == Some(credential) {
                    return Ok(error(
                        "in_use",
                        "This token signs GetKeys in to Cloudflare. Reconnect with another token to roll it.",
                    ));
                }
                request(get(token_url(credential)), json!({"stage": "read"}))
            }
            (Operation::RotateCredential { credential, .. }, Some("read")) => {
                let mut req = get(format!("{}/value", token_url(credential)));
                req.method = "PUT".into();
                req.body = Some(json!({}));
                request(req, json!({"stage": "roll", "token": r}))
            }
            (Operation::RotateCredential { .. }, _) => {
                let Some(value) = r.as_str().filter(|s| !s.is_empty()) else {
                    return Ok(error(
                        "value_unavailable",
                        "Cloudflare rolled the token but did not return the new value. Roll it again in Cloudflare.",
                    ));
                };
                let mut c = token(&call.cursor["token"], "")?;
                c.value = Some(SecretValue {
                    fields: BTreeMap::from([("token".into(), value.into())]),
                });
                done(Outcome::Credential(c))
            }
            _ => error("unsupported", "This Cloudflare operation is unavailable."),
        });
    }
    let (req, cursor) = match &call.operation {
        Operation::Identity => (get(format!("{API}/user")), Value::Null),
        Operation::ListCredentials { resource } if resource.is_empty() => (
            get(format!("{API}/user/tokens/verify")),
            json!({"stage": "current"}),
        ),
        Operation::ListCredentials { .. } => return Ok(done(Outcome::Credentials(vec![]))),
        Operation::ValidateCredential { .. } => {
            let mut r = get(format!("{API}/user/tokens/verify"));
            r.auth = RequestAuth::Candidate;
            (r, Value::Null)
        }
        Operation::RotateCredential { .. } => (
            get(format!("{API}/user/tokens/verify")),
            json!({"stage": "verify"}),
        ),
        Operation::RevokeCredential { credential, .. } => {
            let mut r = get(token_url(credential));
            r.method = "DELETE".into();
            (r, Value::Null)
        }
        _ => {
            return Ok(error(
                "unsupported",
                "This Cloudflare operation is unavailable.",
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
            credential: "t2".into(),
        }
    }
    #[test]
    fn lists_tokens_across_pages_and_marks_the_one_in_use() {
        let list = Operation::ListCredentials {
            resource: String::new(),
        };
        let Step::Request { cursor, .. } = call(
            list.clone(),
            json!({"stage": "current"}),
            Some(json!({"success": true, "result": {"id": "t1", "status": "active"}})),
        ) else {
            panic!()
        };
        let Step::Request { request, cursor } = call(
            list.clone(),
            cursor,
            Some(
                json!({"success": true, "result": [{"id": "t1", "name": "GetKeys", "status": "active", "issued_on": "2024-01-01T00:00:00Z"}], "result_info": {"page": 1, "total_count": 2}}),
            ),
        ) else {
            panic!()
        };
        assert!(request.url.ends_with("page=2"));
        let Step::Complete {
            result: Outcome::Credentials(keys),
        } = call(
            list,
            cursor,
            Some(
                json!({"success": true, "result": [{"id": "t2", "name": "CI", "status": "active", "expires_on": "2030-01-01T00:00:00Z"}], "result_info": {"page": 2, "total_count": 2}}),
            ),
        )
        else {
            panic!()
        };
        assert_eq!(keys[0].credential_type, "sign_in");
        assert_eq!(keys[1].expires_at.as_deref(), Some("2030-01-01T00:00:00Z"));
        assert!(keys.iter().all(|k| k.value.is_none()));
    }
    #[test]
    fn rolling_keeps_the_token_id_and_returns_the_new_value() {
        let Step::Request { cursor, .. } = call(
            rotate(),
            json!({"stage": "verify"}),
            Some(json!({"success": true, "result": {"id": "t1"}})),
        ) else {
            panic!()
        };
        let Step::Request { request, cursor } = call(
            rotate(),
            cursor,
            Some(json!({"success": true, "result": {"id": "t2", "name": "CI"}})),
        ) else {
            panic!()
        };
        assert_eq!(request.method, "PUT");
        assert!(request.url.ends_with("/user/tokens/t2/value"));
        let Step::Complete {
            result: Outcome::Credential(c),
        } = call(
            rotate(),
            cursor,
            Some(json!({"success": true, "result": "new-value"})),
        )
        else {
            panic!()
        };
        assert_eq!((c.id.as_str(), c.name.as_str()), ("t2", "CI"));
        assert!(c.value.is_some());
    }
    #[test]
    fn refuses_to_roll_the_token_in_use() {
        assert!(matches!(
            call(
                rotate(),
                json!({"stage": "verify"}),
                Some(json!({"success": true, "result": {"id": "t2"}}))
            ),
            Step::Error { .. }
        ));
    }
}
