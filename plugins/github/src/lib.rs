use getkeys_plugin_api::*;
use serde_json::{Value, json};

pub const MANIFEST: &str = include_str!("../manifest.json");

fn repo_path(resource: &str) -> Option<String> {
    let (owner, repo) = resource.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some(format!("{}/{}", segment(owner), segment(repo)))
}

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
        if matches!(call.operation, Operation::ValidateCredential { .. }) {
            return Ok(done(Outcome::Done));
        }
        if matches!(call.operation, Operation::WriteSecret { .. })
            && call.cursor["stage"] == "write"
        {
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
            Operation::WriteSecret { resource, name, .. } => {
                let path = repo_path(resource).ok_or("repository")?;
                let mut req = get(format!(
                    "https://api.github.com/repos/{path}/actions/secrets/{}",
                    segment(name)
                ));
                req.method = "PUT".into();
                github_headers(&mut req);
                req.secret = Some(SecretInjection::SealedBox {
                    public_key: v["key"].as_str().ok_or("public key")?.into(),
                    key_id: v["key_id"].as_str().ok_or("key id")?.into(),
                });
                request(req, json!({"stage":"write"}))
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
        Operation::WriteSecret { resource, .. } => {
            let Some(path) = repo_path(resource) else {
                return Ok(error(
                    "invalid_resource",
                    "Select a repository as owner/name.",
                ));
            };
            get(format!(
                "https://api.github.com/repos/{path}/actions/secrets/public-key"
            ))
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
    fn rejects_ambiguous_repository_paths() {
        assert!(repo_path("a/b/c").is_none());
        assert!(repo_path("/b").is_none());
    }
    #[test]
    fn secret_writes_require_sealed_box_injection() {
        let result = handle(Call {
            operation: Operation::WriteSecret {
                resource: "a/b".into(),
                name: "TOKEN".into(),
                environment: "production".into(),
            },
            config: BTreeMap::new(),
            cursor: Value::Null,
            response: Some(HttpResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: json!({"key":"public","key_id":"id"}).to_string(),
            }),
        });
        let Step::Request { request, .. } = result else {
            panic!()
        };
        assert!(matches!(
            request.secret,
            Some(SecretInjection::SealedBox { .. })
        ));
        assert_eq!(request.method, "PUT");
    }
}
