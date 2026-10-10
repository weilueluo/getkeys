use getkeys_plugin_api::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub const MANIFEST: &str = include_str!("../manifest.json");
const KEYS: &str = "https://apikeys.googleapis.com/v2";
const GEMINI: &str = "generativelanguage.googleapis.com";
const PROJECTS: &str = "https://cloudresourcemanager.googleapis.com/v1";
const SERVICES: &str = "https://serviceusage.googleapis.com/v1";
/// Operation polls before giving up on a key that is still being created.
const MAX_POLLS: u64 = 10;
/// Polls, two seconds apart, before giving up on a project that is still being set up.
const PROJECT_POLLS: u64 = 30;

pub fn handle(call: Call) -> Step {
    execute(call).unwrap_or_else(|_| {
        error(
            "invalid_response",
            "Google returned an unexpected response.",
        )
    })
}

fn keys_url(project: &str) -> String {
    format!("{KEYS}/projects/{}/locations/global/keys", segment(project))
}

fn key_url(project: &str, key: &str) -> String {
    format!("{}/{}", keys_url(project), segment(key))
}

fn services(k: &Value) -> Vec<&str> {
    k["restrictions"]["apiTargets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["service"].as_str())
        .collect()
}

/// Keys that can call the Gemini API: unrestricted ones and those allowed to use it.
fn usable(k: &Value) -> bool {
    let s = services(k);
    s.is_empty() || s.contains(&GEMINI)
}

fn key(k: &Value, project: &str) -> Result<RemoteCredential, &'static str> {
    let id = k["name"]
        .as_str()
        .and_then(|n| n.rsplit('/').next())
        .ok_or("name")?;
    let s = services(k);
    let mut details = BTreeMap::from([(
        "apis".to_owned(),
        if s.is_empty() {
            "Any API".to_owned()
        } else {
            s.join(", ")
        },
    )]);
    if let Some(uid) = k["uid"].as_str() {
        details.insert("uid".into(), uid.into());
    }
    let restrictions = k["restrictions"].as_object();
    Ok(RemoteCredential {
        id: id.into(),
        name: k["displayName"].as_str().unwrap_or(id).into(),
        credential_type: "api_key".into(),
        resource: project.into(),
        created_at: k["createTime"].as_str().map(Into::into),
        expires_at: None,
        value: k["keyString"].as_str().map(|v| SecretValue {
            fields: BTreeMap::from([("token".into(), v.into())]),
        }),
        // GetKeys creates keys limited to the Gemini API, so only those are replaced unattended.
        can_rotate: s == [GEMINI] && restrictions.is_some_and(|r| r.len() == 1),
        details,
    })
}

fn with_value(mut c: RemoteCredential, v: &Value) -> Step {
    match v["keyString"].as_str().filter(|s| !s.is_empty()) {
        Some(value) => {
            c.value = Some(SecretValue {
                fields: BTreeMap::from([("token".into(), value.into())]),
            });
            done(Outcome::Credential(c))
        }
        None => error(
            "value_unavailable",
            "Google did not return the key's value.",
        ),
    }
}

/// A Google Cloud project ID from a name: 6 to 30 lowercase letters, digits and hyphens,
/// starting with a letter.
fn project_id(name: &str) -> Option<String> {
    let id = name
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let id = id.get(..30).unwrap_or(&id).trim_end_matches('-');
    (id.len() >= 6 && id.starts_with(|c: char| c.is_ascii_lowercase())).then(|| id.into())
}

fn poll(url: String, mut cursor: Value) -> Step {
    let polls = cursor["polls"].as_u64().unwrap_or(0) + 1;
    cursor["polls"] = json!(polls);
    let mut r = get(url);
    r.wait_ms = 2000;
    request(r, cursor)
}

/// Lists the organizations a project can go in. Personal accounts have none.
fn owners(call: &Call) -> Result<Step, &'static str> {
    let Some(response) = &call.response else {
        return Ok(request(
            post(format!("{PROJECTS}/organizations:search"), json!({})),
            Value::Null,
        ));
    };
    let mut owners = Vec::new();
    if (200..300).contains(&response.status) {
        let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
        for o in v["organizations"].as_array().into_iter().flatten() {
            let id = o["name"]
                .as_str()
                .ok_or("name")?
                .trim_start_matches("organizations/");
            owners.push(Owner {
                id: id.into(),
                name: o["displayName"].as_str().unwrap_or(id).into(),
            });
        }
    }
    owners.push(Owner {
        id: String::new(),
        name: "No organization".into(),
    });
    Ok(done(Outcome::Owners(owners)))
}

/// Creates a project, waits for it, then turns on the Gemini and API Keys APIs in it.
fn create_project(call: &Call, owner: &str, name: &str) -> Result<Step, &'static str> {
    let Some(response) = &call.response else {
        let Some(id) = project_id(name) else {
            return Ok(error(
                "name_unavailable",
                "Google project IDs are 6 to 30 letters, digits or hyphens, starting with a letter. Choose a longer name.",
            ));
        };
        let display: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || " -'!\"".contains(*c))
            .take(30)
            .collect();
        let mut body = json!({
            "projectId": id,
            "name": if display.trim().len() >= 4 { display.trim() } else { &id },
        });
        if !owner.is_empty() {
            body["parent"] = json!({"type": "organization", "id": owner});
        }
        return Ok(request(
            post(format!("{PROJECTS}/projects"), body),
            json!({"stage": "project", "id": id}),
        ));
    };
    let stage = call.cursor["stage"].as_str().unwrap_or("");
    let id = call.cursor["id"].as_str().ok_or("id")?;
    let project = |note: &str| {
        done(Outcome::Resource(Resource {
            id: id.into(),
            name: name.into(),
            kind: "project".into(),
            url: format!("https://console.cloud.google.com/home/dashboard?project={id}"),
            note: note.into(),
            ..Default::default()
        }))
    };
    const API_NOTE: &str = "Turn on the Gemini API in this project before creating keys.";
    if stage == "services" {
        if !(200..300).contains(&response.status) {
            return Ok(project(API_NOTE));
        }
    } else if let Some(err) = name_error(response, "project").or_else(|| provider_error(response)) {
        return Ok(err);
    }
    let v: Value = serde_json::from_str(&response.body).map_err(|_| "json")?;
    let polls = call.cursor["polls"].as_u64().unwrap_or(0);
    if v["done"] != true {
        let op = v["name"].as_str().ok_or("operation")?;
        if polls >= PROJECT_POLLS {
            return Ok(if stage == "project" {
                error(
                    "pending",
                    "Google is still creating the project. Refresh in a minute to see it.",
                )
            } else {
                project(
                    "Google is still turning on the Gemini API. Wait a minute before creating keys.",
                )
            });
        }
        let base = if stage == "project" {
            PROJECTS
        } else {
            SERVICES
        };
        return Ok(poll(format!("{base}/{op}"), call.cursor.clone()));
    }
    if stage == "services" {
        return Ok(project(if v["error"].is_null() { "" } else { API_NOTE }));
    }
    if let Some(message) = v["error"]["message"].as_str() {
        return Ok(error(
            "create_failed",
            &format!("Google could not create the project: {message}"),
        ));
    }
    Ok(request(
        post(
            format!("{SERVICES}/projects/{}/services:batchEnable", segment(id)),
            json!({"serviceIds": [GEMINI, "apikeys.googleapis.com"]}),
        ),
        json!({"stage": "services", "id": id}),
    ))
}

fn execute(call: Call) -> Result<Step, &'static str> {
    match &call.operation {
        Operation::ListOwners => return owners(&call),
        Operation::CreateResource { owner, name, .. } => return create_project(&call, owner, name),
        _ => {}
    }
    if let Some(response) = &call.response {
        // Projects that never enabled the API Keys API have no keys to list.
        if response.status == 403
            && response.body.contains("SERVICE_DISABLED")
            && matches!(call.operation, Operation::ListCredentials { .. })
        {
            return Ok(done(Outcome::Credentials(vec![])));
        }
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
        let next = v["nextPageToken"].as_str().filter(|t| !t.is_empty());
        return Ok(match &call.operation {
            Operation::Identity => done(Outcome::Identity(Identity {
                id: v["sub"].as_str().ok_or("sub")?.into(),
                name: v["email"].as_str().ok_or("email")?.into(),
            })),
            Operation::ListResources => {
                let mut resources: Vec<Resource> =
                    serde_json::from_value(call.cursor["items"].clone()).unwrap_or_default();
                for p in v["projects"].as_array().into_iter().flatten() {
                    let id = p["projectId"].as_str().ok_or("projectId")?;
                    resources.push(Resource {
                        id: id.into(),
                        name: p["name"].as_str().unwrap_or(id).into(),
                        kind: "project".into(),
                        ..Default::default()
                    });
                }
                match next {
                    Some(token) => request(
                        get(format!("{}&pageToken={}", projects_url(), segment(token))),
                        json!({"items": resources}),
                    ),
                    None => done(Outcome::Resources(resources)),
                }
            }
            Operation::ListCredentials { resource } => {
                let mut items = call.cursor["items"].as_array().cloned().unwrap_or_default();
                items.extend(v["keys"].as_array().into_iter().flatten().cloned());
                match next {
                    Some(token) => request(
                        get(format!(
                            "{}?pageSize=300&pageToken={}",
                            keys_url(resource),
                            segment(token)
                        )),
                        json!({"items": items}),
                    ),
                    None => done(Outcome::Credentials(
                        items
                            .iter()
                            .filter(|k| usable(k))
                            .map(|k| key(k, resource))
                            .collect::<Result<_, _>>()?,
                    )),
                }
            }
            Operation::ReadCredential {
                resource,
                credential,
            } => {
                if call.cursor["stage"] == "meta" {
                    request(
                        get(format!("{}/keyString", key_url(resource, credential))),
                        json!({"stage": "value", "key": v}),
                    )
                } else {
                    with_value(key(&call.cursor["key"], resource)?, &v)
                }
            }
            Operation::CreateCredential { resource, .. } => {
                if call.cursor["stage"] == "value" {
                    return Ok(with_value(key(&call.cursor["key"], resource)?, &v));
                }
                if v["done"] != true {
                    let polls = call.cursor["polls"].as_u64().unwrap_or(0);
                    if polls >= MAX_POLLS {
                        return Ok(error(
                            "pending",
                            "Google is still creating the key. Refresh the project in a moment to see it.",
                        ));
                    }
                    let op = v["name"].as_str().ok_or("operation")?;
                    return Ok(request(
                        get(format!("{KEYS}/{op}")),
                        json!({"polls": polls + 1}),
                    ));
                }
                if !v["error"].is_null() {
                    return Ok(error("create_failed", "Google could not create the key."));
                }
                let k = &v["response"];
                let c = key(k, resource)?;
                if c.value.is_some() {
                    done(Outcome::Credential(c))
                } else {
                    request(
                        get(format!("{}/keyString", key_url(resource, &c.id))),
                        json!({"stage": "value", "key": k}),
                    )
                }
            }
            _ => error("unsupported", "This Google operation is unavailable."),
        });
    }
    let (req, cursor) = match &call.operation {
        Operation::Identity => (
            get("https://www.googleapis.com/oauth2/v3/userinfo".into()),
            Value::Null,
        ),
        Operation::ListResources => (get(projects_url()), json!({"items": []})),
        Operation::ListCredentials { resource } => (
            get(format!("{}?pageSize=300", keys_url(resource))),
            json!({"items": []}),
        ),
        Operation::ReadCredential {
            resource,
            credential,
        } => (get(key_url(resource, credential)), json!({"stage": "meta"})),
        Operation::CreateCredential { resource, name } => {
            let mut r = get(keys_url(resource));
            r.method = "POST".into();
            r.body = Some(json!({
                "displayName": name,
                "restrictions": {"apiTargets": [{"service": GEMINI}]}
            }));
            (r, Value::Null)
        }
        Operation::ValidateCredential { .. } => {
            let mut r = get(format!("https://{GEMINI}/v1beta/models?pageSize=1"));
            r.headers
                .insert("x-getkeys-auth-header".into(), "x-goog-api-key".into());
            r.auth = RequestAuth::Candidate;
            (r, Value::Null)
        }
        Operation::RevokeCredential {
            resource,
            credential,
        } => {
            let mut r = get(key_url(resource, credential));
            r.method = "DELETE".into();
            (r, Value::Null)
        }
        _ => {
            return Ok(error(
                "unsupported",
                "This Google operation is unavailable.",
            ));
        }
    };
    Ok(request(req, cursor))
}

fn projects_url() -> String {
    "https://cloudresourcemanager.googleapis.com/v1/projects?filter=lifecycleState%3AACTIVE&pageSize=500".into()
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
    fn reply(operation: Operation, cursor: Value, status: u16, body: Value) -> Step {
        handle(Call {
            operation,
            config: BTreeMap::new(),
            cursor,
            response: Some(HttpResponse {
                status,
                body: body.to_string(),
                headers: BTreeMap::new(),
            }),
        })
    }
    fn start(operation: Operation) -> HttpRequest {
        let Step::Request { request, .. } = handle(Call {
            operation,
            config: BTreeMap::new(),
            cursor: Value::Null,
            response: None,
        }) else {
            panic!()
        };
        request
    }
    fn create() -> Operation {
        Operation::CreateCredential {
            resource: "my-app".into(),
            name: "GetKeys".into(),
        }
    }
    #[test]
    fn lists_only_keys_that_can_call_gemini() {
        let list = Operation::ListCredentials {
            resource: "my-app".into(),
        };
        let Step::Request { request, cursor } = reply(
            list.clone(),
            json!({"items": []}),
            200,
            json!({"keys": [{"name": "projects/1/locations/global/keys/k1", "displayName": "Gemini", "createTime": "2024-05-01T00:00:00Z", "restrictions": {"apiTargets": [{"service": GEMINI}]}}], "nextPageToken": "p2"}),
        ) else {
            panic!()
        };
        assert!(request.url.ends_with("pageSize=300&pageToken=p2"));
        let Step::Complete {
            result: Outcome::Credentials(keys),
        } = reply(
            list,
            cursor,
            200,
            json!({"keys": [
                {"name": "projects/1/locations/global/keys/k2", "displayName": "Any"},
                {"name": "projects/1/locations/global/keys/k3", "restrictions": {"apiTargets": [{"service": "maps-backend.googleapis.com"}]}}
            ]}),
        )
        else {
            panic!()
        };
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].id, "k1");
        assert!(keys[0].can_rotate && !keys[1].can_rotate);
        assert_eq!(keys[1].details["apis"], "Any API");
        assert!(keys.iter().all(|k| k.value.is_none()));
    }
    #[test]
    fn projects_without_the_api_keys_api_have_no_keys() {
        let step = reply(
            Operation::ListCredentials {
                resource: "my-app".into(),
            },
            json!({"items": []}),
            403,
            json!({"error": {"status": "PERMISSION_DENIED", "details": [{"reason": "SERVICE_DISABLED"}]}}),
        );
        assert!(matches!(
            step,
            Step::Complete { result: Outcome::Credentials(k) } if k.is_empty()
        ));
    }
    #[test]
    fn reading_fetches_the_key_string() {
        let read = Operation::ReadCredential {
            resource: "my-app".into(),
            credential: "k1".into(),
        };
        let Step::Request { request, cursor } = reply(
            read.clone(),
            json!({"stage": "meta"}),
            200,
            json!({"name": "projects/1/locations/global/keys/k1", "displayName": "Gemini"}),
        ) else {
            panic!()
        };
        assert!(request.url.ends_with("/keys/k1/keyString"));
        let Step::Complete {
            result: Outcome::Credential(c),
        } = reply(read, cursor, 200, json!({"keyString": "AIza-test"}))
        else {
            panic!()
        };
        assert_eq!(c.name, "Gemini");
        assert!(c.value.is_some());
    }
    #[test]
    fn creation_restricts_the_key_and_waits_for_the_operation() {
        let request = start(create());
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.body.unwrap()["restrictions"]["apiTargets"][0]["service"],
            GEMINI
        );
        let Step::Request { request, cursor } = reply(
            create(),
            Value::Null,
            200,
            json!({"name": "operations/akmf.p7-1", "done": false}),
        ) else {
            panic!()
        };
        assert!(request.url.ends_with("/v2/operations/akmf.p7-1"));
        let Step::Complete {
            result: Outcome::Credential(c),
        } = reply(
            create(),
            cursor,
            200,
            json!({"name": "operations/akmf.p7-1", "done": true, "response": {"name": "projects/1/locations/global/keys/k9", "displayName": "GetKeys", "keyString": "AIza-new", "restrictions": {"apiTargets": [{"service": GEMINI}]}}}),
        )
        else {
            panic!()
        };
        assert_eq!(c.id, "k9");
        assert!(c.value.is_some() && c.can_rotate);
    }
    fn new_project() -> Operation {
        Operation::CreateResource {
            owner: "42".into(),
            name: "My Gemini App".into(),
            options: BTreeMap::new(),
        }
    }
    #[test]
    fn project_ids_come_from_the_name() {
        assert_eq!(
            project_id("My Gemini App").as_deref(),
            Some("my-gemini-app")
        );
        assert_eq!(project_id("app"), None);
        assert_eq!(project_id("1st project"), None);
        assert_eq!(
            project_id("a very long project name that goes on").as_deref(),
            Some("a-very-long-project-name-that")
        );
    }
    #[test]
    fn organizations_come_with_no_organization() {
        let Step::Complete {
            result: Outcome::Owners(o),
        } = reply(
            Operation::ListOwners,
            Value::Null,
            200,
            json!({"organizations": [{"name": "organizations/42", "displayName": "acme.dev"}]}),
        )
        else {
            panic!()
        };
        assert_eq!((o[0].id.as_str(), o[0].name.as_str()), ("42", "acme.dev"));
        assert_eq!(o[1].id, "");
        let Step::Complete {
            result: Outcome::Owners(o),
        } = reply(Operation::ListOwners, Value::Null, 403, json!({}))
        else {
            panic!()
        };
        assert_eq!(o.len(), 1);
    }
    #[test]
    fn new_projects_wait_and_turn_on_gemini() {
        let request = start(new_project());
        assert!(request.url.ends_with("/v1/projects"));
        let body = request.body.unwrap();
        assert_eq!(body["projectId"], "my-gemini-app");
        assert_eq!(body["parent"]["id"], "42");
        let Step::Request { request, cursor } = reply(
            new_project(),
            json!({"stage": "project", "id": "my-gemini-app"}),
            200,
            json!({"name": "operations/cp.1"}),
        ) else {
            panic!()
        };
        assert!(request.url.ends_with("/v1/operations/cp.1"));
        assert_eq!(request.wait_ms, 2000);
        let Step::Request { request, cursor } = reply(
            new_project(),
            cursor,
            200,
            json!({"name": "operations/cp.1", "done": true, "response": {}}),
        ) else {
            panic!()
        };
        assert!(
            request
                .url
                .ends_with("/projects/my-gemini-app/services:batchEnable")
        );
        let Step::Request { request, cursor } = reply(
            new_project(),
            cursor,
            200,
            json!({"name": "operations/acf.2"}),
        ) else {
            panic!()
        };
        assert!(request.url.starts_with(SERVICES));
        let Step::Complete {
            result: Outcome::Resource(r),
        } = reply(
            new_project(),
            cursor,
            200,
            json!({"name": "operations/acf.2", "done": true, "response": {}}),
        )
        else {
            panic!()
        };
        assert_eq!((r.id.as_str(), r.note.as_str()), ("my-gemini-app", ""));
    }
    #[test]
    fn projects_are_kept_when_gemini_cannot_be_turned_on() {
        let Step::Complete {
            result: Outcome::Resource(r),
        } = reply(
            new_project(),
            json!({"stage": "services", "id": "my-gemini-app"}),
            403,
            json!({}),
        )
        else {
            panic!()
        };
        assert!(r.note.contains("Gemini API"));
        assert!(matches!(
            reply(new_project(), json!({"stage": "project", "id": "my-gemini-app"}), 409, json!({})),
            Step::Error { code, .. } if code == "name_unavailable"
        ));
    }
    #[test]
    fn validation_sends_the_key_in_the_gemini_header() {
        let request = start(Operation::ValidateCredential {
            resource: "my-app".into(),
        });
        assert!(matches!(request.auth, RequestAuth::Candidate));
        assert_eq!(request.headers["x-getkeys-auth-header"], "x-goog-api-key");
    }
}
