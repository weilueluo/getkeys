use getkeys_plugin_api::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub const MANIFEST: &str = include_str!("../manifest.json");
const API: &str = "https://api.anthropic.com/v1";
/// Pseudo-resource for keys outside any workspace.
const DEFAULT: &str = "default";

pub fn handle(call: Call) -> Step {
    execute(call).unwrap_or_else(|_| {
        error(
            "invalid_response",
            "Anthropic returned an unexpected response.",
        )
    })
}

/// A request carrying the key in `x-api-key`, as Anthropic expects.
fn api(url: String) -> HttpRequest {
    let mut r = get(url);
    r.headers
        .insert("x-getkeys-auth-header".into(), "x-api-key".into());
    r.headers
        .insert("anthropic-version".into(), "2023-06-01".into());
    r
}

fn list_path(operation: &Operation) -> Option<String> {
    match operation {
        Operation::ListResources => Some(format!("{API}/organizations/workspaces?")),
        Operation::ListCredentials { resource } if resource == DEFAULT => {
            Some(format!("{API}/organizations/api_keys?"))
        }
        Operation::ListCredentials { resource } => Some(format!(
            "{API}/organizations/api_keys?workspace_id={}",
            segment(resource)
        )),
        _ => None,
    }
}

fn page(path: &str, after: Option<&str>) -> HttpRequest {
    let sep = if path.ends_with('?') { "" } else { "&" };
    let mut url = format!("{path}{sep}limit=100");
    if let Some(after) = after {
        url.push_str(&format!("&after_id={}", segment(after)));
    }
    api(url)
}

/// Anthropic never returns a key's value after it is created, so listed keys hold no secret.
fn key(k: &Value, resource: &str) -> Result<RemoteCredential, &'static str> {
    let mut details = BTreeMap::new();
    if let Some(hint) = k["partial_key_hint"].as_str() {
        details.insert("hint".into(), hint.into());
    }
    if let Some(status) = k["status"].as_str() {
        details.insert("status".into(), status.into());
    }
    Ok(RemoteCredential {
        id: k["id"].as_str().ok_or("id")?.into(),
        name: k["name"].as_str().unwrap_or("Anthropic key").into(),
        credential_type: "api_key".into(),
        resource: resource.into(),
        created_at: k["created_at"].as_str().map(Into::into),
        expires_at: k["expires_at"].as_str().map(Into::into),
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
            Operation::ValidateCredential { .. }
                | Operation::DeactivateCredential { .. }
                | Operation::RevokeCredential { .. }
        ) {
            return Ok(done(Outcome::Done));
        }
        let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
        let Some(path) = list_path(&call.operation) else {
            return Ok(done(Outcome::Identity(Identity {
                id: v["id"].as_str().ok_or("id")?.into(),
                name: v["name"]
                    .as_str()
                    .unwrap_or("Anthropic organization")
                    .into(),
            })));
        };
        let mut items = call.cursor["items"].as_array().cloned().unwrap_or_default();
        items.extend(v["data"].as_array().ok_or("data")?.iter().cloned());
        if v["has_more"] == true {
            let after = v["last_id"].as_str().ok_or("last_id")?;
            return Ok(request(page(&path, Some(after)), json!({"items": items})));
        }
        return Ok(match &call.operation {
            Operation::ListResources => {
                let mut resources = vec![Resource {
                    id: DEFAULT.into(),
                    name: "Default workspace".into(),
                    kind: "workspace".into(),
                }];
                for w in &items {
                    resources.push(Resource {
                        id: w["id"].as_str().ok_or("id")?.into(),
                        name: w["name"].as_str().ok_or("name")?.into(),
                        kind: "workspace".into(),
                    });
                }
                done(Outcome::Resources(resources))
            }
            Operation::ListCredentials { resource } => done(Outcome::Credentials(
                items
                    .iter()
                    .filter(|k| k["status"] != "archived")
                    .filter(|k| resource != DEFAULT || k["workspace_id"].is_null())
                    .map(|k| key(k, resource))
                    .collect::<Result<_, _>>()?,
            )),
            _ => error("unsupported", "This Anthropic operation is unavailable."),
        });
    }
    if let Some(path) = list_path(&call.operation) {
        return Ok(request(page(&path, None), json!({"items": []})));
    }
    let req = match &call.operation {
        Operation::Identity => api(format!("{API}/organizations/me")),
        Operation::ValidateCredential { .. } => {
            let mut r = api(format!("{API}/models?limit=1"));
            r.auth = RequestAuth::Candidate;
            r
        }
        Operation::DeactivateCredential { credential, .. }
        | Operation::RevokeCredential { credential, .. } => {
            let status = if matches!(call.operation, Operation::RevokeCredential { .. }) {
                "archived"
            } else {
                "inactive"
            };
            let mut r = api(format!(
                "{API}/organizations/api_keys/{}",
                segment(credential)
            ));
            r.method = "POST".into();
            r.body = Some(json!({"status": status}));
            r
        }
        Operation::Guide { .. } => {
            return Ok(done(Outcome::Guide {
                url: "https://console.anthropic.com/settings/keys".into(),
                message: "Anthropic's API can't create keys. Create one in the Console, paste its value into GetKeys, update consumers, then archive the old key here.".into(),
            }));
        }
        _ => {
            return Ok(error(
                "unsupported",
                "This Anthropic operation is unavailable.",
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
    fn requests_use_the_api_key_header_and_version() {
        let Step::Request { request, .. } = call(Operation::Identity, Value::Null, None) else {
            panic!()
        };
        assert_eq!(request.headers["x-getkeys-auth-header"], "x-api-key");
        assert_eq!(request.headers["anthropic-version"], "2023-06-01");
        assert!(request.url.ends_with("/v1/organizations/me"));
    }
    #[test]
    fn default_workspace_keys_are_those_without_a_workspace() {
        let list = Operation::ListCredentials {
            resource: DEFAULT.into(),
        };
        let Step::Request { request, cursor } = call(
            list.clone(),
            json!({"items": []}),
            Some(
                json!({"data":[{"id":"k1","name":"a","workspace_id":null,"status":"active","partial_key_hint":"sk-ant-api03-R2D...igAA","created_at":"2024-10-30T23:58:27.427722Z"}],"has_more":true,"last_id":"k1"}),
            ),
        ) else {
            panic!()
        };
        assert!(request.url.ends_with("api_keys?limit=100&after_id=k1"));
        let Step::Complete {
            result: Outcome::Credentials(keys),
        } = call(
            list,
            cursor,
            Some(json!({"data":[
                {"id":"k2","name":"b","workspace_id":"wrkspc_1","status":"active"},
                {"id":"k3","name":"c","workspace_id":null,"status":"archived"}
            ],"has_more":false})),
        )
        else {
            panic!()
        };
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].details["hint"], "sk-ant-api03-R2D...igAA");
        assert!(keys[0].value.is_none() && !keys[0].can_rotate);
    }
    #[test]
    fn workspaces_follow_the_default_workspace() {
        let Step::Complete {
            result: Outcome::Resources(r),
        } = call(
            Operation::ListResources,
            json!({"items": []}),
            Some(json!({"data":[{"id":"wrkspc_1","name":"Prod"}],"has_more":false})),
        )
        else {
            panic!()
        };
        assert_eq!((r[0].id.as_str(), r[1].id.as_str()), (DEFAULT, "wrkspc_1"));
    }
    #[test]
    fn revoking_archives_and_deactivating_disables() {
        for (op, status) in [
            (
                Operation::RevokeCredential {
                    resource: DEFAULT.into(),
                    credential: "k1".into(),
                },
                "archived",
            ),
            (
                Operation::DeactivateCredential {
                    resource: DEFAULT.into(),
                    credential: "k1".into(),
                },
                "inactive",
            ),
        ] {
            let Step::Request { request, .. } = call(op, Value::Null, None) else {
                panic!()
            };
            assert_eq!(request.method, "POST");
            assert_eq!(request.body.unwrap()["status"], status);
        }
    }
}
