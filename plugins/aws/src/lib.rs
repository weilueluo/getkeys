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

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PoliciesResponse {
    list_policies_result: PoliciesResult,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PoliciesResult {
    #[serde(default)]
    policies: Policies,
    #[serde(default)]
    is_truncated: bool,
    marker: Option<String>,
}
#[derive(Default, Deserialize)]
struct Policies {
    #[serde(default)]
    member: Vec<Policy>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Policy {
    policy_name: String,
    arn: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GroupsResponse {
    list_groups_result: GroupsResult,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GroupsResult {
    #[serde(default)]
    groups: Groups,
    #[serde(default)]
    is_truncated: bool,
    marker: Option<String>,
}
#[derive(Default, Deserialize)]
struct Groups {
    #[serde(default)]
    member: Vec<Group>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Group {
    group_name: String,
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
/// Lists the account's choices for a new IAM user: its permission policies or its groups.
fn choices(call: &Call, option: &str) -> Result<Step, &'static str> {
    let action = match option {
        "policy" => "ListPolicies",
        "group" => "ListGroups",
        _ => return Ok(error("unsupported", "AWS has no such choice.")),
    };
    let page = |marker: Option<String>| {
        let mut req = iam(action, "");
        req.form.insert("MaxItems".into(), "1000".into());
        if option == "policy" {
            req.form
                .insert("PolicyUsageFilter".into(), "PermissionsPolicy".into());
        }
        if let Some(marker) = marker {
            req.form.insert("Marker".into(), marker);
        }
        req
    };
    let Some(response) = &call.response else {
        return Ok(request(page(None), json!({"choices": []})));
    };
    if let Some(err) = provider_error(response) {
        return Ok(err);
    }
    let mut found: Vec<Choice> =
        serde_json::from_value(call.cursor["choices"].clone()).unwrap_or_default();
    let (truncated, marker) = if option == "policy" {
        let r: PoliciesResponse =
            quick_xml::de::from_str(&response.body).map_err(|_| "policies")?;
        found.extend(r.list_policies_result.policies.member.into_iter().map(|p| {
            let managed = p.arn.contains(":aws:policy/");
            Choice {
                label: if managed {
                    format!("{} (AWS managed)", p.policy_name)
                } else {
                    p.policy_name
                },
                value: p.arn,
            }
        }));
        (
            r.list_policies_result.is_truncated,
            r.list_policies_result.marker,
        )
    } else {
        let r: GroupsResponse = quick_xml::de::from_str(&response.body).map_err(|_| "groups")?;
        found.extend(
            r.list_groups_result
                .groups
                .member
                .into_iter()
                .map(|g| Choice {
                    value: g.group_name.clone(),
                    label: g.group_name,
                }),
        );
        (
            r.list_groups_result.is_truncated,
            r.list_groups_result.marker,
        )
    };
    if truncated {
        return Ok(request(
            page(Some(marker.ok_or("marker")?)),
            json!({"choices": found}),
        ));
    }
    // The account's own policies come before the many AWS managed ones.
    found.sort_by_key(|c| (c.value.contains(":aws:policy/"), c.label.to_lowercase()));
    let none = if option == "policy" {
        "No permissions"
    } else {
        "No group"
    };
    found.insert(
        0,
        Choice {
            value: String::new(),
            label: none.into(),
        },
    );
    Ok(done(Outcome::Choices(found)))
}

/// IAM user names are up to 64 letters, digits and `+=,.@_-`.
fn valid_user_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+=,.@_-".contains(c))
}

/// Creates an IAM user, then gives it the chosen policy and group.
fn create_user(call: &Call, name: &str, options: &BTreeMap<String, String>) -> Step {
    let option = |key: &str| options.get(key).filter(|v| !v.is_empty()).cloned();
    let stage = call.cursor["stage"].as_str().unwrap_or("");
    if let Some(response) = &call.response {
        if stage == "user"
            && let Some(err) = name_error(response, "IAM user")
        {
            return err;
        }
        if let Some(err) = provider_error(response) {
            if stage == "user" {
                return err;
            }
            let what = if stage == "policy" {
                "its permissions"
            } else {
                "its group"
            };
            let Step::Error { code, message } = err else {
                return err;
            };
            return error(
                &code,
                &format!("AWS created the IAM user {name}, but couldn't add {what}: {message}"),
            );
        }
    } else if !valid_user_name(name) {
        return error(
            "name_unavailable",
            "IAM user names are up to 64 letters, digits and + = , . @ _ -",
        );
    }
    let next = match (call.response.is_some(), stage) {
        (false, _) => Some(("user", iam("CreateUser", name))),
        (true, "user") => option("policy")
            .map(|arn| {
                let mut r = iam("AttachUserPolicy", name);
                r.form.insert("PolicyArn".into(), arn);
                ("policy", r)
            })
            .or_else(|| group(name, option("group"))),
        (true, "policy") => group(name, option("group")),
        _ => None,
    };
    match next {
        Some((stage, req)) => request(req, json!({"stage": stage})),
        None => done(Outcome::Resource(Resource {
            id: name.into(),
            name: name.into(),
            kind: "iam_user".into(),
            url: format!(
                "https://console.aws.amazon.com/iam/home#/users/details/{}",
                segment(name)
            ),
            ..Default::default()
        })),
    }
}

fn group(name: &str, group: Option<String>) -> Option<(&'static str, HttpRequest)> {
    group.map(|g| {
        let mut r = iam("AddUserToGroup", name);
        r.form.insert("GroupName".into(), g);
        ("group", r)
    })
}

pub fn handle(call: Call) -> Step {
    execute(call)
        .unwrap_or_else(|_| error("invalid_response", "AWS returned an unexpected response."))
}
fn execute(call: Call) -> Result<Step, &'static str> {
    match &call.operation {
        Operation::ListChoices { option } => return choices(&call, option),
        Operation::CreateResource { name, options, .. } => {
            return Ok(create_user(&call, name, options));
        }
        _ => {}
    }
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
    fn reply(operation: Operation, cursor: serde_json::Value, body: Option<&str>) -> Step {
        handle(Call {
            operation,
            config: BTreeMap::new(),
            cursor,
            response: body.map(|b| HttpResponse {
                status: 200,
                body: b.into(),
                headers: BTreeMap::new(),
            }),
        })
    }
    #[test]
    fn policies_are_listed_page_by_page_with_the_account_s_own_first() {
        let op = Operation::ListChoices {
            option: "policy".into(),
        };
        let Step::Request { request, cursor } = reply(op.clone(), serde_json::Value::Null, None)
        else {
            panic!()
        };
        assert_eq!(request.form["Action"], "ListPolicies");
        let Step::Request { request, cursor } = reply(
            op.clone(),
            cursor,
            Some(
                "<ListPoliciesResponse><ListPoliciesResult><IsTruncated>true</IsTruncated><Marker>m1</Marker><Policies><member><PolicyName>ReadOnlyAccess</PolicyName><Arn>arn:aws:iam::aws:policy/ReadOnlyAccess</Arn></member></Policies></ListPoliciesResult></ListPoliciesResponse>",
            ),
        ) else {
            panic!()
        };
        assert_eq!(request.form["Marker"], "m1");
        let Step::Complete {
            result: Outcome::Choices(c),
        } = reply(
            op,
            cursor,
            Some(
                "<ListPoliciesResponse><ListPoliciesResult><IsTruncated>false</IsTruncated><Policies><member><PolicyName>app-s3</PolicyName><Arn>arn:aws:iam::123456789012:policy/app-s3</Arn></member></Policies></ListPoliciesResult></ListPoliciesResponse>",
            ),
        )
        else {
            panic!()
        };
        let labels: Vec<_> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            ["No permissions", "app-s3", "ReadOnlyAccess (AWS managed)"]
        );
        assert_eq!(c[0].value, "");
    }
    #[test]
    fn groups_include_no_group() {
        let Step::Complete {
            result: Outcome::Choices(c),
        } = reply(
            Operation::ListChoices {
                option: "group".into(),
            },
            json!({"choices": []}),
            Some(
                "<ListGroupsResponse><ListGroupsResult><IsTruncated>false</IsTruncated><Groups><member><GroupName>developers</GroupName></member></Groups></ListGroupsResult></ListGroupsResponse>",
            ),
        )
        else {
            panic!()
        };
        assert_eq!(
            (c[0].label.as_str(), c[1].value.as_str()),
            ("No group", "developers")
        );
    }
    #[test]
    fn new_users_get_the_chosen_policy_and_group() {
        let op = Operation::CreateResource {
            owner: String::new(),
            name: "ci-bot".into(),
            options: BTreeMap::from([
                (
                    "policy".into(),
                    "arn:aws:iam::aws:policy/ReadOnlyAccess".into(),
                ),
                ("group".into(), "developers".into()),
            ]),
        };
        let Step::Request { request, cursor } = reply(op.clone(), serde_json::Value::Null, None)
        else {
            panic!()
        };
        assert_eq!(request.form["Action"], "CreateUser");
        assert_eq!(request.form["UserName"], "ci-bot");
        let Step::Request { request, cursor } = reply(op.clone(), cursor, Some("<ok/>")) else {
            panic!()
        };
        assert_eq!(request.form["Action"], "AttachUserPolicy");
        assert_eq!(
            request.form["PolicyArn"],
            "arn:aws:iam::aws:policy/ReadOnlyAccess"
        );
        let Step::Request { request, cursor } = reply(op.clone(), cursor, Some("<ok/>")) else {
            panic!()
        };
        assert_eq!(request.form["Action"], "AddUserToGroup");
        assert_eq!(request.form["GroupName"], "developers");
        let Step::Complete {
            result: Outcome::Resource(r),
        } = reply(op, cursor, Some("<ok/>"))
        else {
            panic!()
        };
        assert_eq!((r.id.as_str(), r.kind.as_str()), ("ci-bot", "iam_user"));
    }
    #[test]
    fn new_users_without_choices_are_done_once_created() {
        let op = Operation::CreateResource {
            owner: String::new(),
            name: "bot".into(),
            options: BTreeMap::from([("policy".into(), String::new())]),
        };
        assert!(matches!(
            reply(op, json!({"stage": "user"}), Some("<ok/>")),
            Step::Complete {
                result: Outcome::Resource(_)
            }
        ));
        let bad = Operation::CreateResource {
            owner: String::new(),
            name: "has space".into(),
            options: BTreeMap::new(),
        };
        assert!(
            matches!(reply(bad, serde_json::Value::Null, None), Step::Error { code, .. } if code == "name_unavailable")
        );
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
