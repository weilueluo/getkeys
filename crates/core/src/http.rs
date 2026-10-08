use crate::model::AuthMaterial;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use getkeys_plugin_api::*;
use std::{
    io::Read,
    time::{Duration, SystemTime},
};
use zeroize::Zeroizing;

pub fn validate_url(value: &str, hosts: &[String]) -> Result<url::Url> {
    let url = url::Url::parse(value).context("Invalid provider URL")?;
    ensure!(
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "Provider requests must use an unambiguous HTTPS URL"
    );
    let host = url.host_str().context("Provider URL has no host")?;
    ensure!(
        host.parse::<std::net::IpAddr>().is_err(),
        "Direct IP destinations are not permitted"
    );
    ensure!(
        hosts.iter().any(|allowed| {
            if let Some(suffix) = allowed.strip_prefix("*.") {
                host.ends_with(&format!(".{suffix}")) && host.len() > suffix.len() + 1
            } else {
                host == allowed
            }
        }),
        "Plugin requested a host outside its declared permissions"
    );
    Ok(url)
}

pub fn bounded_response(response: reqwest::blocking::Response, max: usize) -> Result<Vec<u8>> {
    ensure!(
        response.content_length().is_none_or(|n| n <= max as u64),
        "Provider response exceeds the size limit"
    );
    let mut out = Vec::new();
    response.take(max as u64 + 1).read_to_end(&mut out)?;
    ensure!(out.len() <= max, "Provider response exceeds the size limit");
    Ok(out)
}

pub struct Transport {
    client: reqwest::blocking::Client,
}
impl Transport {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(25))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("GetKeys/0.1")
                .build()?,
        })
    }
    pub fn send(
        &self,
        mut input: HttpRequest,
        hosts: &[String],
        connection: &AuthMaterial,
        candidate: Option<&SecretValue>,
        injected: Option<&str>,
    ) -> Result<HttpResponse> {
        validate_url(&input.url, hosts)?;
        ensure!(
            ["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&input.method.as_str()),
            "Unsupported HTTP method"
        );
        ensure!(
            !input
                .headers
                .keys()
                .any(|h| ["authorization", "cookie", "host"]
                    .contains(&h.to_ascii_lowercase().as_str())),
            "Plugin cannot override authentication or routing headers"
        );
        let service = input.headers.remove("x-getkeys-aws-service");
        let auth_header = input.headers.remove("x-getkeys-auth-header");
        if let Some(injection) = input.secret {
            let secret =
                injected.context("This operation was not granted a consumer credential")?;
            match injection {
                SecretInjection::JsonField { field } => {
                    let object = input
                        .body
                        .get_or_insert_with(|| serde_json::json!({}))
                        .as_object_mut()
                        .context("Secret payload must be a JSON object")?;
                    object.insert(field, secret.into());
                }
                SecretInjection::SealedBox { public_key, key_id } => {
                    let key: [u8; 32] = STANDARD
                        .decode(public_key)?
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("Invalid destination encryption key"))?;
                    let encrypted = crypto_box::PublicKey::from(key)
                        .seal(&mut rand::rngs::OsRng, secret.as_bytes())
                        .map_err(|_| anyhow::anyhow!("Unable to encrypt the destination secret"))?;
                    input.body = Some(
                        serde_json::json!({"encrypted_value":STANDARD.encode(encrypted),"key_id":key_id}),
                    );
                }
            }
        }
        let candidate_auth;
        let auth = match input.auth {
            RequestAuth::Connection => Some(connection),
            RequestAuth::None => None,
            RequestAuth::Candidate => {
                candidate_auth =
                    auth_from_secret(candidate.context("No candidate credential was granted")?)?;
                Some(&candidate_auth)
            }
        };
        let body = Zeroizing::new(if let Some(json) = input.body {
            input
                .headers
                .insert("Content-Type".into(), "application/json".into());
            serde_json::to_vec(&json)?
        } else if !input.form.is_empty() {
            input.headers.insert(
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            );
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(&input.form)
                .finish()
                .into_bytes()
        } else {
            Vec::new()
        });
        if let Some(auth) = auth {
            match auth {
                AuthMaterial::Bearer { token, .. } => {
                    if let Some(header) = auth_header {
                        ensure!(header == "apikey", "Unsupported authentication header");
                        input.headers.insert(header, token.clone());
                    } else {
                        input
                            .headers
                            .insert("Authorization".into(), format!("Bearer {token}"));
                    }
                }
                AuthMaterial::Aws {
                    access_key_id,
                    secret_access_key,
                    session_token,
                    ..
                } => {
                    let service = service.context("AWS signing service is required")?;
                    ensure!(
                        ["iam", "sts"].contains(&service.as_str()),
                        "Unsupported AWS signing service"
                    );
                    let identity = aws_credential_types::Credentials::new(
                        access_key_id,
                        secret_access_key,
                        session_token.clone(),
                        None,
                        "GetKeys",
                    )
                    .into();
                    let params = aws_sigv4::sign::v4::SigningParams::builder()
                        .identity(&identity)
                        .region("us-east-1")
                        .name(&service)
                        .time(SystemTime::now())
                        .settings(aws_sigv4::http_request::SigningSettings::default())
                        .build()?
                        .into();
                    let signable = aws_sigv4::http_request::SignableRequest::new(
                        &input.method,
                        &input.url,
                        input.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                        aws_sigv4::http_request::SignableBody::Bytes(&body),
                    )?;
                    let (instructions, _) =
                        aws_sigv4::http_request::sign(signable, &params)?.into_parts();
                    for (name, value) in instructions.headers() {
                        input.headers.insert(name.into(), value.into());
                    }
                }
            }
        }
        let mut req = self.client.request(input.method.parse()?, &input.url);
        for (name, value) in &input.headers {
            req = req.header(name, value);
        }
        let response=req.body(body.to_vec()).send().map_err(|_|anyhow::anyhow!("Provider request failed. Its outcome may be uncertain; refresh before retrying a write."))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter(|(name, _)| ["link", "retry-after"].contains(&name.as_str()))
            .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_owned())))
            .collect();
        let body = String::from_utf8(bounded_response(response, 4 * 1024 * 1024)?)
            .context("Provider returned non-UTF-8 content")?;
        Ok(HttpResponse {
            status,
            body,
            headers,
        })
    }
}

pub fn auth_from_secret(value: &SecretValue) -> Result<AuthMaterial> {
    if let Some(token) = value.fields.get("token") {
        ensure!(!token.trim().is_empty(), "Token is empty");
        return Ok(AuthMaterial::Bearer {
            token: token.clone(),
            refresh_token: None,
            expires_at: None,
            broker_registration: None,
        });
    }
    Ok(AuthMaterial::Aws {
        access_key_id: value
            .fields
            .get("access_key_id")
            .context("Missing access key ID")?
            .clone(),
        secret_access_key: value
            .fields
            .get("secret_access_key")
            .context("Missing secret access key")?
            .clone(),
        session_token: value.fields.get("session_token").cloned(),
        expires_at: None,
        sso: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn provider_host_permissions_are_exact_and_boundary_aware() {
        let hosts = vec!["api.github.com".into(), "*.supabase.co".into()];
        for good in [
            "https://api.github.com/user",
            "https://project.supabase.co/auth/v1/settings",
        ] {
            assert!(validate_url(good, &hosts).is_ok());
        }
        for bad in [
            "http://api.github.com/user",
            "https://api.github.com.evil.com",
            "https://evil-supabase.co",
            "https://localhost/",
            "https://127.0.0.1",
            "https://user:pass@api.github.com",
            "https://api.github.com:444/",
        ] {
            assert!(validate_url(bad, &hosts).is_err(), "{bad}");
        }
    }
    #[test]
    fn credential_bundles_cannot_silently_drop_required_fields() {
        assert!(
            auth_from_secret(&SecretValue {
                fields: BTreeMap::from([("access_key_id".into(), "id".into())])
            })
            .is_err()
        );
    }
}
