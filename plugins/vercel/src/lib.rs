use getkeys_plugin_api::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub const MANIFEST: &str = include_str!("../manifest.json");
pub fn handle(call: Call) -> Step {
    execute(call).unwrap_or_else(|_| {
        error(
            "invalid_response",
            "Vercel returned an unexpected response.",
        )
    })
}
fn team_url(path: &str, config: &BTreeMap<String, String>) -> String {
    let mut url = format!("https://api.vercel.com{path}");
    if let Some(team) = config.get("team_id").filter(|s| !s.is_empty()) {
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(&format!("teamId={}", segment(team)));
    }
    url
}
fn execute(call: Call) -> Result<Step, &'static str> {
    if let Some(response) = &call.response {
        if let Some(error) = provider_error(response) {
            return Ok(error);
        }
        if matches!(call.operation, Operation::ValidateCredential { .. }) {
            return Ok(done(Outcome::Done));
        }
        let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
        return Ok(match &call.operation {
            Operation::Identity => done(Outcome::Identity(Identity {
                id: v["user"]["id"].as_str().ok_or("id")?.into(),
                name: v["user"]["username"].as_str().ok_or("username")?.into(),
            })),
            Operation::ListResources => {
                let mut resources: Vec<Resource> =
                    serde_json::from_value(call.cursor["resources"].clone()).unwrap_or_default();
                for p in v["projects"].as_array().ok_or("projects")? {
                    resources.push(Resource {
                        id: p["id"].as_str().ok_or("id")?.into(),
                        name: p["name"].as_str().ok_or("name")?.into(),
                        kind: "project".into(),
                    });
                }
                if let Some(next) = v["pagination"]["next"].as_u64() {
                    request(
                        get(team_url(
                            &format!("/v9/projects?limit=100&until={next}"),
                            &call.config,
                        )),
                        json!({"resources":resources}),
                    )
                } else {
                    done(Outcome::Resources(resources))
                }
            }
            Operation::WriteSecret { .. } => {
                if v["failed"].as_array().is_some_and(|f| !f.is_empty()) || !v["error"].is_null() {
                    error(
                        "write_failed",
                        "Vercel did not accept the environment variable. The consumer was not updated.",
                    )
                } else {
                    done(Outcome::Done)
                }
            }
            _ => error("unsupported", "This Vercel operation is unavailable."),
        });
    }
    let mut req=match &call.operation {
        Operation::Identity=>get(team_url("/v2/user",&call.config)),
        Operation::ListResources=>get(team_url("/v9/projects?limit=100",&call.config)),
        Operation::ListCredentials { .. }=>return Ok(done(Outcome::Credentials(vec![]))),
        Operation::Guide { .. }=>return Ok(done(Outcome::Guide { url:"https://vercel.com/account/settings/tokens".into(),message:"Create a scoped token in Vercel, import its one-time value, update consumers, and revoke the old token in Vercel after verification. Integration OAuth cannot manage personal tokens.".into() })),
        Operation::ValidateCredential { resource }=>get(team_url(&format!("/v9/projects/{}",segment(resource)),&call.config)),
        Operation::WriteSecret { resource,.. }=>get(team_url(&format!("/v10/projects/{}/env?upsert=true",segment(resource)),&call.config)),
        _=>return Ok(error("guided_operation","Use Vercel's dashboard to create or revoke personal tokens.")),
    };
    match call.operation {
        Operation::ValidateCredential { .. } => req.auth = RequestAuth::Candidate,
        Operation::WriteSecret {
            name, environment, ..
        } => {
            if !["production", "preview"].contains(&environment.as_str()) {
                return Ok(error(
                    "invalid_environment",
                    "Secret destinations support production and preview environments.",
                ));
            }
            req.method = "POST".into();
            req.body = Some(json!({"key":name,"type":"sensitive","target":[environment]}));
            req.secret = Some(SecretInjection::JsonField {
                field: "value".into(),
            });
        }
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
    fn write() -> Call {
        Call {
            operation: Operation::WriteSecret {
                resource: "p".into(),
                name: "TOKEN".into(),
                environment: "production".into(),
            },
            config: BTreeMap::from([("team_id".into(), "team_1".into())]),
            cursor: Value::Null,
            response: None,
        }
    }
    #[test]
    fn writes_preserve_environment_and_use_secret_type() {
        let Step::Request { request, .. } = handle(write()) else {
            panic!()
        };
        assert!(request.url.contains("teamId=team_1"));
        assert_eq!(request.body.unwrap()["type"], "sensitive");
        assert!(request.secret.is_some());
    }
    #[test]
    fn http_success_with_failed_write_is_not_success() {
        let mut c = write();
        c.response = Some(HttpResponse {
            status: 200,
            body: json!({"failed":[{"error":"duplicate"}]}).to_string(),
            headers: BTreeMap::new(),
        });
        assert!(matches!(handle(c), Step::Error { .. }));
    }
}
