use getkeys_plugin_api::*;
use serde_json::{Value, json};

pub const MANIFEST: &str = include_str!("../manifest.json");

pub fn handle(call: Call) -> Step {
    execute(call).unwrap_or_else(|_| {
        error(
            "invalid_response",
            "GitHub returned an unexpected response.",
        )
    })
}

fn execute(call: Call) -> Result<Step, &'static str> {
    if let Operation::Guide { resource } = &call.operation {
        let owner = resource.split('/').next().unwrap_or("");
        return Ok(done(Outcome::Guide { url: format!("https://github.com/settings/personal-access-tokens/new?name=GetKeys&target_name={}&expires_in=30", segment(owner)), message: "Create a fine-grained token in GitHub, then import it here. Existing PAT values cannot be recovered through the API.".into() }));
    }
    if matches!(call.operation, Operation::ListCredentials { .. }) {
        return Ok(done(Outcome::Credentials(vec![])));
    }
    if let Some(response) = &call.response {
        if let Some(err) = provider_error(response) {
            return Ok(err);
        }
        if matches!(
            call.operation,
            Operation::ValidateCredential { .. } | Operation::RevokeCredential { .. }
        ) {
            return Ok(done(Outcome::Done));
        }
        let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
        return Ok(match &call.operation {
            Operation::Identity => done(Outcome::Identity(Identity {
                id: v["id"].as_u64().ok_or("id")?.to_string(),
                name: v["login"].as_str().ok_or("login")?.into(),
            })),
            Operation::ListResources => {
                let repos = v.as_array().ok_or("repos")?;
                let mut resources: Vec<Resource> =
                    serde_json::from_value(call.cursor["resources"].clone()).unwrap_or_default();
                for repo in repos {
                    let id = repo["full_name"].as_str().ok_or("full_name")?;
                    resources.push(Resource {
                        id: id.into(),
                        name: id.into(),
                        kind: "repository".into(),
                        ..Default::default()
                    });
                }
                if repos.len() == 100 {
                    let page = call.cursor["page"].as_u64().unwrap_or(1) + 1;
                    let mut r = get(format!(
                        "https://api.github.com/user/repos?per_page=100&page={page}&sort=full_name"
                    ));
                    github_headers(&mut r);
                    request(r, json!({"page":page,"resources":resources}))
                } else {
                    done(Outcome::Resources(resources))
                }
            }
            _ => error(
                "unsupported",
                "This GitHub operation requires a guided workflow.",
            ),
        });
    }
    let mut req = match &call.operation {
        Operation::Identity => get("https://api.github.com/user".into()),
        Operation::ValidateCredential { .. } => {
            let mut r = get("https://api.github.com/user".into());
            r.auth = RequestAuth::Candidate;
            r
        }
        Operation::ListResources => {
            get("https://api.github.com/user/repos?per_page=100&page=1&sort=full_name".into())
        }
        // GitHub revokes any token given its value. The request carries no sign-in: the
        // endpoint refuses authenticated calls.
        Operation::RevokeCredential { .. } => {
            let mut r = post(
                "https://api.github.com/credentials/revoke".into(),
                json!({"credentials": []}),
            );
            r.auth = RequestAuth::None;
            r.secret = Some(SecretInjection::JsonArray {
                field: "credentials".into(),
            });
            r
        }
        _ => {
            return Ok(error(
                "unsupported",
                "GitHub PAT creation and replacement use the guided workflow.",
            ));
        }
    };
    github_headers(&mut req);
    Ok(request(req, json!({"page":1,"resources":[]})))
}

fn github_headers(req: &mut HttpRequest) {
    req.headers
        .insert("Accept".into(), "application/vnd.github+json".into());
    req.headers
        .insert("X-GitHub-Api-Version".into(), "2026-03-10".into());
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
    use std::collections::BTreeMap;
    #[test]
    fn pat_creation_is_explicitly_guided() {
        let result = handle(Call {
            operation: Operation::Guide {
                resource: "owner/repo".into(),
            },
            config: BTreeMap::new(),
            cursor: Value::Null,
            response: None,
        });
        let Step::Complete {
            result: Outcome::Guide { url, .. },
        } = result
        else {
            panic!()
        };
        assert!(url.contains("target_name=owner"));
    }
    #[test]
    fn tokens_are_revoked_by_value_without_signing_in() {
        let call = |response| Call {
            operation: Operation::RevokeCredential {
                resource: "owner/repo".into(),
                credential: "import-1".into(),
            },
            config: BTreeMap::new(),
            cursor: Value::Null,
            response,
        };
        let Step::Request { request, .. } = handle(call(None)) else {
            panic!()
        };
        assert_eq!(request.url, "https://api.github.com/credentials/revoke");
        assert!(matches!(request.auth, RequestAuth::None));
        assert!(
            matches!(request.secret, Some(SecretInjection::JsonArray { field }) if field == "credentials")
        );
        let accepted = HttpResponse {
            status: 202,
            body: "{}".into(),
            headers: BTreeMap::new(),
        };
        assert!(matches!(
            handle(call(Some(accepted))),
            Step::Complete {
                result: Outcome::Done
            }
        ));
    }
}
