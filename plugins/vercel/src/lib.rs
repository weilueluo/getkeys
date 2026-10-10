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
const TOKENS: &str = "https://api.vercel.com/v5/user/tokens";

/// RFC 3339 time of a Unix timestamp in milliseconds.
fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
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

/// An account access token. Vercel shows a token's value only when it is created.
fn token(t: &Value, current: &str) -> Result<RemoteCredential, &'static str> {
    let id = t["id"].as_str().ok_or("token id")?;
    let teams: Vec<&str> = t["scopes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s["teamId"].as_str())
        .collect();
    let mut details = BTreeMap::from([(
        "scope".to_owned(),
        if teams.is_empty() {
            "Full account".to_owned()
        } else {
            teams.join(", ")
        },
    )]);
    if let Some(origin) = t["origin"].as_str() {
        details.insert("origin".into(), origin.into());
    }
    if let Some(at) = t["activeAt"].as_i64() {
        details.insert("last_used".into(), iso(at));
    }
    Ok(RemoteCredential {
        id: id.into(),
        name: t["name"].as_str().unwrap_or("Vercel token").into(),
        credential_type: if id == current {
            "sign_in"
        } else {
            "access_token"
        }
        .into(),
        resource: String::new(),
        created_at: t["createdAt"].as_i64().map(iso),
        expires_at: t["expiresAt"].as_i64().map(iso),
        value: None,
        // A new token gets full account access, so only full-account tokens rotate unattended.
        can_rotate: teams.is_empty() && t["expiresAt"].is_null(),
        details,
    })
}

fn execute(call: Call) -> Result<Step, &'static str> {
    if let Some(response) = &call.response {
        if let Some(error) = provider_error(response) {
            return Ok(error);
        }
        match &call.operation {
            Operation::ValidateCredential { resource } if !resource.is_empty() => {
                return Ok(done(Outcome::Done));
            }
            Operation::RevokeCredential { .. } => return Ok(done(Outcome::Done)),
            _ => {}
        }
        let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
        return Ok(match &call.operation {
            Operation::ListCredentials { .. } if call.cursor["stage"] == "current" => request(
                get(TOKENS.into()),
                json!({"current": v["token"]["id"], "tokens": []}),
            ),
            Operation::ListCredentials { .. } => {
                let mut tokens = call.cursor["tokens"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                tokens.extend(v["tokens"].as_array().ok_or("tokens")?.iter().cloned());
                if let Some(next) = v["pagination"]["next"].as_i64() {
                    request(
                        get(format!("{TOKENS}?until={next}")),
                        json!({"current": call.cursor["current"], "tokens": tokens}),
                    )
                } else {
                    let current = call.cursor["current"].as_str().unwrap_or_default();
                    done(Outcome::Credentials(
                        tokens
                            .iter()
                            .map(|t| token(t, current))
                            .collect::<Result<_, _>>()?,
                    ))
                }
            }
            Operation::CreateCredential { .. } => {
                let mut c = token(&v["token"], "")?;
                let Some(value) = v["bearerToken"].as_str().filter(|s| !s.is_empty()) else {
                    return Ok(error(
                        "value_unavailable",
                        "Vercel created the token but did not return it. Delete it in Vercel before retrying.",
                    ));
                };
                c.value = Some(SecretValue {
                    fields: BTreeMap::from([("token".into(), value.into())]),
                });
                done(Outcome::Credential(c))
            }
            Operation::Identity | Operation::ValidateCredential { .. } => {
                done(Outcome::Identity(Identity {
                    id: v["user"]["id"].as_str().ok_or("id")?.into(),
                    name: v["user"]["username"].as_str().ok_or("username")?.into(),
                }))
            }
            Operation::ListResources => {
                let mut resources: Vec<Resource> =
                    serde_json::from_value(call.cursor["resources"].clone()).unwrap_or_default();
                for p in v["projects"].as_array().ok_or("projects")? {
                    resources.push(Resource {
                        id: p["id"].as_str().ok_or("id")?.into(),
                        name: p["name"].as_str().ok_or("name")?.into(),
                        kind: "project".into(),
                        ..Default::default()
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
            _ => error("unsupported", "This Vercel operation is unavailable."),
        });
    }
    let mut req=match &call.operation {
        Operation::Identity=>get(team_url("/v2/user",&call.config)),
        Operation::ListResources=>get(team_url("/v9/projects?limit=100",&call.config)),
        Operation::ListCredentials { resource } if resource.is_empty()=>return Ok(request(get(format!("{TOKENS}/current")),json!({"stage":"current"}))),
        Operation::ListCredentials { .. }=>return Ok(done(Outcome::Credentials(vec![]))),
        Operation::CreateCredential { resource, name } if resource.is_empty()=>{let mut r=get("https://api.vercel.com/v3/user/tokens".into());r.method="POST".into();r.body=Some(json!({"name":name}));r}
        Operation::RevokeCredential { resource, credential } if resource.is_empty()=>{let mut r=get(format!("https://api.vercel.com/v3/user/tokens/{}",segment(credential)));r.method="DELETE".into();r}
        Operation::ValidateCredential { resource } if resource.is_empty()=>get("https://api.vercel.com/v2/user".into()),
        Operation::Guide { .. }=>return Ok(done(Outcome::Guide { url:"https://vercel.com/account/settings/tokens".into(),message:"Create a scoped token in Vercel, save its one-time value here, swap it in where the old one is used, then revoke the old token in Vercel. Integration OAuth cannot manage personal tokens.".into() })),
        Operation::ValidateCredential { resource }=>get(team_url(&format!("/v9/projects/{}",segment(resource)),&call.config)),
        _=>return Ok(error("guided_operation","Use Vercel's dashboard to create or revoke personal tokens.")),
    };
    if matches!(call.operation, Operation::ValidateCredential { .. }) {
        req.auth = RequestAuth::Candidate;
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
    fn respond(operation: Operation, cursor: Value, body: Value) -> Step {
        handle(Call {
            operation,
            config: BTreeMap::new(),
            cursor,
            response: Some(HttpResponse {
                status: 200,
                body: body.to_string(),
                headers: BTreeMap::new(),
            }),
        })
    }
    #[test]
    fn lists_account_tokens_and_marks_the_one_in_use() {
        let list = Operation::ListCredentials {
            resource: String::new(),
        };
        let Step::Request { cursor, .. } = respond(
            list.clone(),
            json!({"stage":"current"}),
            json!({"token":{"id":"t1"}}),
        ) else {
            panic!()
        };
        let Step::Complete {
            result: Outcome::Credentials(keys),
        } = respond(
            list,
            cursor,
            json!({"tokens":[
                {"id":"t1","name":"cli","createdAt":0,"scopes":[{"type":"user"}]},
                {"id":"t2","name":"ci","expiresAt":1_700_000_000_000_i64,"scopes":[{"type":"team","teamId":"team_1"}]}
            ]}),
        )
        else {
            panic!()
        };
        assert_eq!(keys[0].credential_type, "sign_in");
        assert_eq!(keys[0].created_at.as_deref(), Some("1970-01-01T00:00:00Z"));
        assert!(keys[0].can_rotate && !keys[1].can_rotate);
        assert_eq!(keys[1].expires_at.as_deref(), Some("2023-11-14T22:13:20Z"));
        assert!(keys.iter().all(|k| k.value.is_none()));
    }
    #[test]
    fn created_tokens_carry_their_one_time_value() {
        let Step::Complete {
            result: Outcome::Credential(c),
        } = respond(
            Operation::CreateCredential {
                resource: String::new(),
                name: "GetKeys".into(),
            },
            Value::Null,
            json!({"token":{"id":"t3","name":"GetKeys"},"bearerToken":"secret"}),
        )
        else {
            panic!()
        };
        assert_eq!(c.id, "t3");
        assert!(c.value.is_some());
    }
}
