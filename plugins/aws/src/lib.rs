use getkeys_plugin_api::*;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

pub const MANIFEST: &str = include_str!("../manifest.json");

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UsersResponse {
    list_users_result: UsersResult,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UsersResult {
    users: Users,
    #[serde(default)]
    is_truncated: bool,
    marker: Option<String>,
}
#[derive(Default, Deserialize)]
struct Users {
    #[serde(default)]
    member: Vec<User>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct User {
    user_name: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct KeysResponse {
    list_access_keys_result: KeysResult,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct KeysResult {
    access_key_metadata: Keys,
}
#[derive(Default, Deserialize)]
struct Keys {
    #[serde(default)]
    member: Vec<Key>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Key {
    access_key_id: String,
    create_date: Option<String>,
    status: Option<String>,
    secret_access_key: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CreateResponse {
    create_access_key_result: CreateResult,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CreateResult {
    access_key: Key,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct IdentityResponse {
    get_caller_identity_result: Caller,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Caller {
    account: String,
    arn: String,
}

fn iam(action: &str, resource: &str) -> HttpRequest {
    let mut req = get("https://iam.amazonaws.com/".into());
    req.method = "POST".into();
    req.form.insert("Action".into(), action.into());
    req.form.insert("Version".into(), "2010-05-08".into());
    if !resource.is_empty() {
        req.form.insert("UserName".into(), resource.into());
    }
    req.headers
        .insert("x-getkeys-aws-service".into(), "iam".into());
    req
}
fn sts(candidate: bool) -> HttpRequest {
    let mut req = get("https://sts.amazonaws.com/".into());
    req.method = "POST".into();
    req.form = BTreeMap::from([
        ("Action".into(), "GetCallerIdentity".into()),
        ("Version".into(), "2011-06-15".into()),
    ]);
    req.headers
        .insert("x-getkeys-aws-service".into(), "sts".into());
    if candidate {
        req.auth = RequestAuth::Candidate;
    }
    req
}
fn credential(key: Key, resource: &str) -> RemoteCredential {
    let value = key.secret_access_key.map(|s| SecretValue {
        fields: BTreeMap::from([
            ("access_key_id".into(), key.access_key_id.clone()),
            ("secret_access_key".into(), s),
        ]),
    });
    let mut details = BTreeMap::from([("access_key_id".into(), key.access_key_id.clone())]);
    if let Some(status) = key.status {
        details.insert("status".into(), status);
    }
    RemoteCredential {
        id: key.access_key_id.clone(),
        name: key.access_key_id,
        credential_type: "iam_access_key".into(),
        resource: resource.into(),
        created_at: key.create_date,
        expires_at: None,
        value,
        can_rotate: true,
        details,
    }
}
pub fn handle(call: Call) -> Step {
    execute(call)
        .unwrap_or_else(|_| error("invalid_response", "AWS returned an unexpected response."))
}
fn execute(call: Call) -> Result<Step, &'static str> {
    if let Some(response) = &call.response {
        if let Some(err) = provider_error(response) {
            return Ok(err);
        }
        return Ok(match &call.operation {
            Operation::Identity => {
                let r: IdentityResponse =
                    quick_xml::de::from_str(&response.body).map_err(|_| "identity")?;
                done(Outcome::Identity(Identity {
                    id: r.get_caller_identity_result.account,
                    name: r.get_caller_identity_result.arn,
                }))
            }
            Operation::ListResources => {
                let r: UsersResponse =
                    quick_xml::de::from_str(&response.body).map_err(|_| "users")?;
                let mut resources: Vec<Resource> =
                    serde_json::from_value(call.cursor["resources"].clone()).unwrap_or_default();
                resources.extend(
                    r.list_users_result
                        .users
                        .member
                        .into_iter()
                        .map(|u| Resource {
                            id: u.user_name.clone(),
                            name: u.user_name,
                            kind: "iam_user".into(),
                            ..Default::default()
                        }),
                );
                if r.list_users_result.is_truncated {
                    let mut req = iam("ListUsers", "");
                    req.form
                        .insert("Marker".into(), r.list_users_result.marker.ok_or("marker")?);
                    request(req, json!({"resources":resources}))
                } else {
                    done(Outcome::Resources(resources))
                }
            }
            Operation::ListCredentials { resource } => {
                let r: KeysResponse =
                    quick_xml::de::from_str(&response.body).map_err(|_| "keys")?;
                done(Outcome::Credentials(
                    r.list_access_keys_result
                        .access_key_metadata
                        .member
                        .into_iter()
                        .map(|k| credential(k, resource))
                        .collect(),
                ))
            }
            Operation::CreateCredential { resource, .. } if call.cursor["stage"] == "preflight" => {
                let r: KeysResponse =
                    quick_xml::de::from_str(&response.body).map_err(|_| "keys")?;
                if r.list_access_keys_result.access_key_metadata.member.len() >= 2 {
                    error(
                        "key_limit",
                        "This IAM user already has two access keys. Review them in AWS before creating a replacement.",
                    )
                } else {
                    request(iam("CreateAccessKey", resource), json!({"stage":"created"}))
                }
            }
            Operation::CreateCredential { resource, .. } => {
                let r: CreateResponse =
                    quick_xml::de::from_str(&response.body).map_err(|_| "create")?;
                let c = credential(r.create_access_key_result.access_key, resource);
                if c.value.is_none() {
                    error(
                        "value_unavailable",
                        "AWS did not return the new secret. Inspect the user before retrying.",
                    )
                } else {
                    done(Outcome::Credential(c))
                }
            }
            Operation::ValidateCredential { resource } => {
                let r: IdentityResponse =
                    quick_xml::de::from_str(&response.body).map_err(|_| "identity")?;
                let identity = r.get_caller_identity_result;
                if !identity.arn.contains(":user/")
                    || identity.arn.rsplit('/').next() != Some(resource)
                {
                    error(
                        "wrong_user",
                        "This access key belongs to a different IAM user.",
                    )
                } else {
                    done(Outcome::Identity(Identity {
                        id: identity.account,
                        name: identity.arn,
                    }))
                }
            }
            Operation::DeactivateCredential { .. } | Operation::RevokeCredential { .. } => {
                done(Outcome::Done)
            }
            _ => error(
                "unsupported",
                "AWS access key secrets can only be captured at creation or imported.",
            ),
        });
    }
    let req = match &call.operation {
        Operation::Identity => sts(false),
        Operation::ValidateCredential { .. } => sts(true),
        Operation::ListResources => iam("ListUsers", ""),
        Operation::ListCredentials { resource } | Operation::CreateCredential { resource, .. } => {
            iam("ListAccessKeys", resource)
        }
        Operation::DeactivateCredential {
            resource,
            credential,
        } => {
            let mut r = iam("UpdateAccessKey", resource);
            r.form.insert("AccessKeyId".into(), credential.clone());
            r.form.insert("Status".into(), "Inactive".into());
            r
        }
        Operation::RevokeCredential {
            resource,
            credential,
        } => {
            let mut r = iam("DeleteAccessKey", resource);
            r.form.insert("AccessKeyId".into(), credential.clone());
            r
        }
        _ => {
            return Ok(error(
                "unsupported",
                "AWS access key secrets cannot be retrieved after creation.",
            ));
        }
    };
    Ok(request(req, json!({"stage":"preflight","resources":[]})))
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
    #[test]
    fn imported_keys_must_belong_to_the_selected_iam_user() {
        for (arn, valid) in [
            ("arn:aws:iam::123456789012:user/team/expected", true),
            ("arn:aws:iam::123456789012:user/other", false),
            (
                "arn:aws:sts::123456789012:assumed-role/role/expected",
                false,
            ),
        ] {
            let result = handle(Call {
                operation: Operation::ValidateCredential {
                    resource: "expected".into(),
                },
                config: BTreeMap::new(),
                cursor: serde_json::Value::Null,
                response: Some(HttpResponse {
                    status: 200,
                    headers: BTreeMap::new(),
                    body: format!(
                        "<GetCallerIdentityResponse><GetCallerIdentityResult><Account>123456789012</Account><Arn>{arn}</Arn></GetCallerIdentityResult></GetCallerIdentityResponse>"
                    ),
                }),
            });
            if valid {
                assert!(
                    matches!(result, Step::Complete { result: Outcome::Identity(Identity { id, .. }) } if id == "123456789012")
                );
            } else {
                assert!(matches!(result, Step::Error { code, .. } if code == "wrong_user"));
            }
        }
    }
    #[test]
    fn refuses_creation_when_both_key_slots_are_used() {
        let body = "<ListAccessKeysResponse><ListAccessKeysResult><AccessKeyMetadata><member><AccessKeyId>A</AccessKeyId></member><member><AccessKeyId>B</AccessKeyId></member></AccessKeyMetadata></ListAccessKeysResult></ListAccessKeysResponse>";
        let result = handle(Call {
            operation: Operation::CreateCredential {
                resource: "user".into(),
                name: "new".into(),
            },
            config: BTreeMap::new(),
            cursor: json!({"stage":"preflight"}),
            response: Some(HttpResponse {
                status: 200,
                body: body.into(),
                headers: BTreeMap::new(),
            }),
        });
        assert!(matches!(result,Step::Error { code,.. } if code=="key_limit"));
    }
    #[test]
    fn captures_both_parts_of_a_new_access_key() {
        let body = "<CreateAccessKeyResponse><CreateAccessKeyResult><AccessKey><AccessKeyId>AKIAEXAMPLE</AccessKeyId><SecretAccessKey>secret</SecretAccessKey></AccessKey></CreateAccessKeyResult></CreateAccessKeyResponse>";
        let result = handle(Call {
            operation: Operation::CreateCredential {
                resource: "user".into(),
                name: "new".into(),
            },
            config: BTreeMap::new(),
            cursor: json!({"stage":"created"}),
            response: Some(HttpResponse {
                status: 200,
                body: body.into(),
                headers: BTreeMap::new(),
            }),
        });
        let Step::Complete {
            result: Outcome::Credential(c),
        } = result
        else {
            panic!()
        };
        assert_eq!(c.value.unwrap().fields["access_key_id"], "AKIAEXAMPLE");
    }
}
