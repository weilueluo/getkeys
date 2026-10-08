use crate::{model::*, new_id, now, vault::Vault};
use anyhow::{Context, Result, bail, ensure};
use getkeys_plugin_api::{AuthMethod, Manifest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};

#[derive(Serialize, Deserialize)]
struct Flow {
    plugin_id: String,
    config: BTreeMap<String, String>,
    expires_at: i64,
    next_poll: i64,
    interval: u64,
    state: FlowState,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
enum FlowState {
    Device {
        token_url: String,
        client_id: String,
        device_code: String,
    },
    Broker {
        broker: String,
        registration: String,
        session_id: String,
        claim_secret: String,
    },
    AwsDevice {
        region: String,
        client_id: String,
        client_secret: String,
        device_code: String,
    },
    AwsRoles {
        region: String,
        client_id: String,
        client_secret: String,
        access_token: String,
        refresh_token: Option<String>,
        roles: Vec<RoleChoice>,
    },
    Authorized {
        auth: AuthMaterial,
    },
}

pub enum PollResult {
    Progress(AuthProgress),
    Authorized {
        plugin_id: String,
        config: BTreeMap<String, String>,
        auth: AuthMaterial,
    },
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("GetKeys/0.1")
        .build()?)
}
fn json_response(response: reqwest::blocking::Response) -> Result<(u16, Value)> {
    let status = response.status().as_u16();
    let bytes = crate::http::bounded_response(response, 1024 * 1024)?;
    Ok((
        status,
        serde_json::from_slice(&bytes)
            .context("Authentication server returned an invalid response")?,
    ))
}
fn field(v: &Value, key: &str) -> Result<String> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .with_context(|| format!("Authentication response is missing {key}"))
}
pub fn broker_url() -> Result<String> {
    let value = std::env::var("GETKEYS_BROKER_URL")
        .context("Configure GETKEYS_BROKER_URL to connect this provider. See docs/setup.md.")?;
    let u = url::Url::parse(&value)?;
    let local = matches!(u.host_str(), Some("127.0.0.1" | "localhost"));
    ensure!(
        (u.scheme() == "https" || (local && u.scheme() == "http"))
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none(),
        "OAuth broker must use HTTPS (HTTP is allowed only on localhost for development)"
    );
    Ok(value.trim_end_matches('/').into())
}
fn aws_region(value: &str) -> Result<()> {
    let parts: Vec<_> = value.split('-').collect();
    ensure!(
        parts.len() == 3
            && parts[0].len() == 2
            && parts[0].bytes().all(|b| b.is_ascii_lowercase())
            && parts[1].bytes().all(|b| b.is_ascii_lowercase())
            && parts[2].parse::<u8>().is_ok(),
        "Use a commercial AWS region such as eu-west-2"
    );
    Ok(())
}

pub fn begin(
    vault: &Vault,
    m: &Manifest,
    config: BTreeMap<String, String>,
    existing_id: Option<&str>,
) -> Result<AuthProgress> {
    for field in &m.fields {
        ensure!(
            !field.required || config.get(&field.key).is_some_and(|s| !s.trim().is_empty()),
            "{} is required",
            field.label
        );
    }
    ensure!(
        config.keys().all(|k| m.fields.iter().any(|f| f.key == *k)),
        "Unexpected provider configuration field"
    );
    let id = existing_id.map(str::to_owned).unwrap_or(new_id()?);
    let http = client()?;
    let (state, url, user_code, expires, interval) = match &m.auth {
        AuthMethod::Device {
            device_url,
            token_url,
            client_id_env,
            scopes,
        } => {
            crate::http::validate_url(device_url, &m.hosts)?;
            crate::http::validate_url(token_url, &m.hosts)?;
            let client_id = std::env::var(client_id_env).ok().filter(|v| !v.trim().is_empty()).with_context(|| {
                format!(
                    "Configure {client_id_env} with your OAuth app client ID. See docs/setup.md."
                )
            })?;
            let (status, v) = json_response(
                http.post(device_url)
                    .header("Accept", "application/json")
                    .form(&[("client_id", &client_id), ("scope", scopes)])
                    .send()
                    .context("Unable to start browser authorization")?,
            )?;
            ensure!(
                status == 200 && !v["device_code"].is_null(),
                "Provider did not accept the OAuth app configuration"
            );
            (
                FlowState::Device {
                    token_url: token_url.clone(),
                    client_id,
                    device_code: field(&v, "device_code")?,
                },
                field(&v, "verification_uri")?,
                Some(field(&v, "user_code")?),
                v["expires_in"].as_i64().unwrap_or(600),
                v["interval"].as_u64().unwrap_or(5),
            )
        }
        AuthMethod::Broker { registration } => {
            let broker = broker_url()?;
            let claim_secret = new_id()?;
            let (status, v) = json_response(
                http.post(format!("{broker}/sessions"))
                    .json(&json!({"provider":registration,"claim_secret":claim_secret}))
                    .send()
                    .context("OAuth broker is unavailable")?,
            )?;
            ensure!(
                status == 200,
                "OAuth registration is unavailable on the broker"
            );
            (
                FlowState::Broker {
                    broker,
                    registration: registration.clone(),
                    session_id: field(&v, "id")?,
                    claim_secret,
                },
                field(&v, "authorize_url")?,
                None,
                600,
                2,
            )
        }
        AuthMethod::AwsSso => {
            let region = config
                .get("region")
                .context("SSO region is required")?
                .clone();
            aws_region(&region)?;
            let start_url = config
                .get("start_url")
                .context("SSO start URL is required")?;
            crate::http::validate_url(start_url, &["*.awsapps.com".into()])?;
            let base = format!("https://oidc.{region}.amazonaws.com");
            let (status,registration)=json_response(http.post(format!("{base}/client/register")).json(&json!({"clientName":"GetKeys","clientType":"public","scopes":["sso:account:access"]})).send()?)?;
            ensure!(status == 200, "AWS client registration failed");
            let client_id = field(&registration, "clientId")?;
            let client_secret = field(&registration, "clientSecret")?;
            let (status,v)=json_response(http.post(format!("{base}/device_authorization")).json(&json!({"clientId":client_id,"clientSecret":client_secret,"startUrl":start_url})).send()?)?;
            ensure!(
                status == 200,
                "AWS did not accept this Identity Center start URL"
            );
            let url = v["verificationUriComplete"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or(field(&v, "verificationUri")?);
            (
                FlowState::AwsDevice {
                    region,
                    client_id,
                    client_secret,
                    device_code: field(&v, "deviceCode")?,
                },
                url,
                Some(field(&v, "userCode")?),
                v["expiresIn"].as_i64().unwrap_or(600),
                v["interval"].as_u64().unwrap_or(5),
            )
        }
    };
    let parsed = url::Url::parse(&url)?;
    ensure!(
        parsed.scheme() == "https",
        "Provider authorization must open an HTTPS page"
    );
    let flow = Flow {
        plugin_id: m.id.clone(),
        config,
        expires_at: now() + expires.min(1800),
        next_poll: now() + interval as i64,
        interval,
        state,
    };
    vault.put("auth_flow", &id, &flow)?;
    Ok(AuthProgress::Browser {
        flow_id: id,
        url,
        user_code,
        interval,
    })
}

pub fn poll(vault: &Vault, id: &str) -> Result<PollResult> {
    let mut flow: Flow = vault.get("auth_flow", id)?;
    ensure!(
        flow.expires_at > now(),
        "Authorization expired. Connect again."
    );
    if let FlowState::Authorized { auth } = &flow.state {
        return Ok(PollResult::Authorized {
            plugin_id: flow.plugin_id.clone(),
            config: flow.config.clone(),
            auth: auth.clone(),
        });
    }
    if let FlowState::AwsRoles { roles, .. } = &flow.state {
        return Ok(PollResult::Progress(AuthProgress::SelectRole {
            flow_id: id.into(),
            roles: roles.clone(),
        }));
    }
    if now() < flow.next_poll {
        return Ok(PollResult::Progress(AuthProgress::Pending {
            interval: (flow.next_poll - now()) as u64,
        }));
    }
    flow.next_poll = now() + flow.interval as i64;
    vault.put("auth_flow", id, &flow)?;
    let http = client()?;
    let (status,v)=match &flow.state {
        FlowState::Device { token_url,client_id,device_code }=>json_response(http.post(token_url).header("Accept","application/json").form(&[("client_id",client_id.as_str()),("device_code",device_code.as_str()),("grant_type","urn:ietf:params:oauth:grant-type:device_code")]).send()?)?,
        FlowState::Broker { broker,session_id,claim_secret,.. }=>json_response(http.post(format!("{broker}/sessions/{session_id}/claim")).json(&json!({"claim_secret":claim_secret})).send()?)?,
        FlowState::AwsDevice { region,client_id,client_secret,device_code }=>json_response(http.post(format!("https://oidc.{region}.amazonaws.com/token")).json(&json!({"clientId":client_id,"clientSecret":client_secret,"deviceCode":device_code,"grantType":"urn:ietf:params:oauth:grant-type:device_code"})).send()?)?,
        FlowState::AwsRoles { .. }|FlowState::Authorized { .. }=>unreachable!(),
    };
    let error = v["error"].as_str().or(v["__type"].as_str()).unwrap_or("");
    if status == 202 || ["authorization_pending", "AuthorizationPendingException"].contains(&error)
    {
        return Ok(PollResult::Progress(AuthProgress::Pending {
            interval: flow.interval,
        }));
    }
    if ["slow_down", "SlowDownException"].contains(&error) {
        flow.interval += 5;
        flow.next_poll = now() + flow.interval as i64;
        vault.put("auth_flow", id, &flow)?;
        return Ok(PollResult::Progress(AuthProgress::Pending {
            interval: flow.interval,
        }));
    }
    if status >= 400 || !error.is_empty() {
        vault.remove("auth_flow", id)?;
        bail!("Authorization was denied, expired, or could not be completed. Connect again.");
    }
    if let FlowState::AwsDevice {
        region,
        client_id,
        client_secret,
        ..
    } = flow.state
    {
        let access_token = field(&v, "accessToken")?;
        let roles = list_roles(&http, &region, &access_token)?;
        ensure!(
            !roles.is_empty(),
            "No AWS account roles are assigned to this identity"
        );
        flow.state = FlowState::AwsRoles {
            region,
            client_id,
            client_secret,
            access_token,
            refresh_token: v["refreshToken"].as_str().map(str::to_owned),
            roles: roles.clone(),
        };
        vault.put("auth_flow", id, &flow)?;
        return Ok(PollResult::Progress(AuthProgress::SelectRole {
            flow_id: id.into(),
            roles,
        }));
    }
    if let Some(team) = v["team_id"].as_str() {
        flow.config.insert("team_id".into(), team.into());
    }
    let registration = match &flow.state {
        FlowState::Broker { registration, .. } => Some(registration.clone()),
        _ => None,
    };
    let auth = AuthMaterial::Bearer {
        token: field(&v, "access_token")?,
        refresh_token: v["refresh_token"].as_str().map(str::to_owned),
        expires_at: v["expires_in"].as_i64().map(|s| now() + s),
        broker_registration: registration,
    };
    flow.state = FlowState::Authorized { auth: auth.clone() };
    vault.put("auth_flow", id, &flow)?;
    Ok(PollResult::Authorized {
        plugin_id: flow.plugin_id,
        config: flow.config,
        auth,
    })
}

fn list_roles(
    http: &reqwest::blocking::Client,
    region: &str,
    token: &str,
) -> Result<Vec<RoleChoice>> {
    let base = format!("https://portal.sso.{region}.amazonaws.com");
    let mut roles = Vec::new();
    let mut next = String::new();
    for _ in 0..100 {
        let mut req = http
            .get(format!("{base}/assignment/accounts"))
            .header("x-amz-sso_bearer_token", token)
            .query(&[("max_result", "100")]);
        if !next.is_empty() {
            req = req.query(&[("next_token", &next)]);
        }
        let (status, v) = json_response(req.send()?)?;
        ensure!(status == 200, "Unable to list AWS accounts");
        for account in v["accountList"]
            .as_array()
            .context("Invalid account response")?
        {
            let id = field(account, "accountId")?;
            let name = field(account, "accountName")?;
            let mut role_next = String::new();
            for page in 0..100 {
                let mut request = http
                    .get(format!("{base}/assignment/roles"))
                    .header("x-amz-sso_bearer_token", token)
                    .query(&[("account_id", id.as_str()), ("max_result", "100")]);
                if !role_next.is_empty() {
                    request = request.query(&[("next_token", &role_next)]);
                }
                let (status, r) = json_response(request.send()?)?;
                ensure!(status == 200, "Unable to list AWS roles");
                for role in r["roleList"].as_array().context("Invalid role response")? {
                    roles.push(RoleChoice {
                        account_id: id.clone(),
                        account_name: name.clone(),
                        role: field(role, "roleName")?,
                    });
                }
                role_next = r["nextToken"].as_str().unwrap_or("").into();
                if role_next.is_empty() {
                    break;
                }
                ensure!(page < 99, "AWS role list exceeded the page limit");
            }
        }
        next = v["nextToken"].as_str().unwrap_or("").into();
        if next.is_empty() {
            return Ok(roles);
        }
    }
    bail!("AWS account list exceeded the page limit")
}

fn role_credentials(session: &AwsSession) -> Result<Value> {
    let (status, v) = json_response(
        client()?
            .get(format!(
                "https://portal.sso.{}.amazonaws.com/federation/credentials",
                session.region
            ))
            .header("x-amz-sso_bearer_token", &session.access_token)
            .query(&[
                ("account_id", &session.account_id),
                ("role_name", &session.role_name),
            ])
            .send()?,
    )?;
    ensure!(
        status == 200,
        "AWS session expired or no longer permits this role. Reconnect."
    );
    Ok(v["roleCredentials"].clone())
}
fn aws_material(session: AwsSession, v: Value) -> Result<AuthMaterial> {
    Ok(AuthMaterial::Aws {
        access_key_id: field(&v, "accessKeyId")?,
        secret_access_key: field(&v, "secretAccessKey")?,
        session_token: Some(field(&v, "sessionToken")?),
        expires_at: v["expiration"].as_i64().map(|n| n / 1000),
        sso: Some(session),
    })
}
pub fn choose_role(vault: &Vault, id: &str, account: &str, role: &str) -> Result<PollResult> {
    let mut flow: Flow = vault.get("auth_flow", id)?;
    ensure!(
        flow.expires_at > now(),
        "Authorization expired. Connect again."
    );
    let FlowState::AwsRoles {
        region,
        client_id,
        client_secret,
        access_token,
        refresh_token,
        roles,
    } = flow.state
    else {
        bail!("This authorization is not waiting for a role")
    };
    ensure!(
        roles
            .iter()
            .any(|r| r.account_id == account && r.role == role),
        "The selected role was not granted to this identity"
    );
    let session = AwsSession {
        region,
        client_id,
        client_secret,
        access_token,
        refresh_token,
        account_id: account.into(),
        role_name: role.into(),
    };
    let v = role_credentials(&session)?;
    let auth = aws_material(session, v)?;
    flow.state = FlowState::Authorized { auth: auth.clone() };
    vault.put("auth_flow", id, &flow)?;
    Ok(PollResult::Authorized {
        plugin_id: flow.plugin_id,
        config: flow.config,
        auth,
    })
}

pub fn refresh(connection: &mut Connection, manifest: &Manifest) -> Result<()> {
    let expires = match &connection.auth {
        AuthMaterial::Bearer { expires_at, .. } | AuthMaterial::Aws { expires_at, .. } => {
            *expires_at
        }
    };
    if expires.is_none_or(|e| e > now() + 60) {
        return Ok(());
    }
    match &connection.auth {
        AuthMaterial::Bearer {
            refresh_token: Some(refresh),
            broker_registration,
            ..
        } => {
            let (status, v) = if let Some(registration) = broker_registration {
                json_response(
                    client()?
                        .post(format!("{}/refresh", broker_url()?))
                        .json(&json!({"provider":registration,"refresh_token":refresh}))
                        .send()?,
                )?
            } else {
                let AuthMethod::Device {
                    token_url,
                    client_id_env,
                    ..
                } = &manifest.auth
                else {
                    bail!("Reconnect to renew this credential")
                };
                let client_id = std::env::var(client_id_env)?;
                json_response(
                    client()?
                        .post(token_url)
                        .header("Accept", "application/json")
                        .form(&[
                            ("grant_type", "refresh_token"),
                            ("refresh_token", refresh.as_str()),
                            ("client_id", client_id.as_str()),
                        ])
                        .send()?,
                )?
            };
            ensure!(
                status == 200 && v["error"].is_null(),
                "Connection renewal failed. Reconnect this account."
            );
            connection.auth = AuthMaterial::Bearer {
                token: field(&v, "access_token")?,
                refresh_token: v["refresh_token"]
                    .as_str()
                    .map(str::to_owned)
                    .or(Some(refresh.clone())),
                expires_at: v["expires_in"].as_i64().map(|n| now() + n),
                broker_registration: broker_registration.clone(),
            };
        }
        AuthMaterial::Aws {
            sso: Some(session), ..
        } => {
            let mut session = session.clone();
            let v = match role_credentials(&session) {
                Ok(v) => v,
                Err(_) => {
                    let refresh = session
                        .refresh_token
                        .as_ref()
                        .context("AWS session expired. Reconnect this account.")?;
                    let (status,v)=json_response(client()?.post(format!("https://oidc.{}.amazonaws.com/token",session.region)).json(&json!({"clientId":session.client_id,"clientSecret":session.client_secret,"grantType":"refresh_token","refreshToken":refresh})).send()?)?;
                    ensure!(
                        status == 200,
                        "AWS session expired. Reconnect this account."
                    );
                    session.access_token = field(&v, "accessToken")?;
                    session.refresh_token = v["refreshToken"]
                        .as_str()
                        .map(str::to_owned)
                        .or(session.refresh_token);
                    role_credentials(&session)?
                }
            };
            connection.auth = aws_material(session, v)?;
        }
        _ => bail!("This connection expired. Reconnect this account."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn region_cannot_inject_a_host_or_path() {
        for invalid in ["eu-west-2.evil.com", "../local", "us-east-1/secret", ""] {
            assert!(aws_region(invalid).is_err());
        }
        assert!(aws_region("eu-west-2").is_ok());
    }
    #[test]
    fn expiry_returns_reauthentication_instead_of_reusing_a_token() {
        let mut c = Connection {
            identity_id: "test".into(),
            view: ConnectionView {
                id: "c".into(),
                plugin_id: "p".into(),
                name: "test".into(),
                state: ConnectionState::Connected,
                config: BTreeMap::new(),
            },
            auth: AuthMaterial::Bearer {
                token: "secret".into(),
                expires_at: Some(0),
                refresh_token: None,
                broker_registration: None,
            },
        };
        let m: Manifest =
            serde_json::from_str(include_str!("../../../plugins/github/manifest.json")).unwrap();
        assert!(refresh(&mut c, &m).is_err());
    }
}
