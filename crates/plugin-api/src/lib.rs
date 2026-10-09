use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use ts_rs::TS;

pub const API_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub api_version: u32,
    pub description: String,
    pub homepage: String,
    pub hosts: Vec<String>,
    pub auth: AuthMethod,
    pub operations: Vec<OperationKind>,
    pub fields: Vec<ConfigField>,
    /// Fields of keys listed under a resource, in display order.
    pub credential_fields: Vec<ConfigField>,
    /// Fields of the account's own sign-in key, which can also be pasted to connect.
    #[serde(default)]
    pub account_fields: Vec<ConfigField>,
    /// Provider page where a user creates a token to paste.
    #[serde(default)]
    pub token_page: String,
    /// The account's own access tokens are listed, created and revoked under resource `""`.
    /// A token the plugin lists with credential type `sign_in` is the one the connection uses.
    #[serde(default)]
    pub account_credentials: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthMethod {
    Device {
        device_url: String,
        token_url: String,
        client_id_env: String,
        scopes: String,
    },
    Broker {
        registration: String,
    },
    AwsSso,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
pub struct ConfigField {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub required: bool,
    /// Masked until revealed. Values not marked secret are shown as they are.
    #[serde(default = "secret_by_default")]
    pub secret: bool,
    /// Environment variable used when exporting this field, if any.
    #[serde(default)]
    pub env: String,
}

fn secret_by_default() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Identity,
    ListResources,
    ListCredentials,
    ReadCredential,
    CreateCredential,
    ValidateCredential,
    DeactivateCredential,
    RevokeCredential,
    WriteSecret,
    Guide,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Identity,
    ListResources,
    ListCredentials {
        resource: String,
    },
    ReadCredential {
        resource: String,
        credential: String,
    },
    CreateCredential {
        resource: String,
        name: String,
    },
    ValidateCredential {
        resource: String,
    },
    DeactivateCredential {
        resource: String,
        credential: String,
    },
    RevokeCredential {
        resource: String,
        credential: String,
    },
    WriteSecret {
        resource: String,
        name: String,
        environment: String,
    },
    Guide {
        resource: String,
    },
}

impl Operation {
    pub fn kind(&self) -> OperationKind {
        match self {
            Self::Identity => OperationKind::Identity,
            Self::ListResources => OperationKind::ListResources,
            Self::ListCredentials { .. } => OperationKind::ListCredentials,
            Self::ReadCredential { .. } => OperationKind::ReadCredential,
            Self::CreateCredential { .. } => OperationKind::CreateCredential,
            Self::ValidateCredential { .. } => OperationKind::ValidateCredential,
            Self::DeactivateCredential { .. } => OperationKind::DeactivateCredential,
            Self::RevokeCredential { .. } => OperationKind::RevokeCredential,
            Self::WriteSecret { .. } => OperationKind::WriteSecret,
            Self::Guide { .. } => OperationKind::Guide,
        }
    }
    pub fn mutates(&self) -> bool {
        matches!(
            self,
            Self::CreateCredential { .. }
                | Self::DeactivateCredential { .. }
                | Self::RevokeCredential { .. }
                | Self::WriteSecret { .. }
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Call {
    pub operation: Operation,
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    #[serde(default)]
    pub cursor: Value,
    pub response: Option<HttpResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    Request { request: HttpRequest, cursor: Value },
    Complete { result: Outcome },
    Error { code: String, message: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub body: Option<Value>,
    #[serde(default)]
    pub form: BTreeMap<String, String>,
    pub auth: RequestAuth,
    pub secret: Option<SecretInjection>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecretInjection {
    JsonField { field: String },
    SealedBox { public_key: String, key_id: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestAuth {
    Connection,
    Candidate,
    None,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Outcome {
    Identity(Identity),
    Resources(Vec<Resource>),
    Credentials(Vec<RemoteCredential>),
    Credential(RemoteCredential),
    Done,
    Guide { url: String, message: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct Identity {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct Resource {
    pub id: String,
    pub name: String,
    pub kind: String,
}

// Secret-bearing transport objects deliberately do not implement Debug or TS.
#[derive(Clone, Deserialize, Serialize)]
pub struct SecretValue {
    pub fields: BTreeMap<String, String>,
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue([redacted])")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RemoteCredential {
    pub id: String,
    pub name: String,
    pub credential_type: String,
    pub resource: String,
    pub created_at: Option<String>,
    pub expires_at: Option<String>,
    pub value: Option<SecretValue>,
    pub can_rotate: bool,
    /// Non-secret facts about the key, such as its ID, status, or endpoint.
    #[serde(default)]
    pub details: BTreeMap<String, String>,
}

pub fn get(url: String) -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        url,
        headers: BTreeMap::new(),
        body: None,
        form: BTreeMap::new(),
        auth: RequestAuth::Connection,
        secret: None,
    }
}

pub fn error(code: &str, message: &str) -> Step {
    Step::Error {
        code: code.into(),
        message: message.into(),
    }
}
pub fn done(result: Outcome) -> Step {
    Step::Complete { result }
}
pub fn request(request: HttpRequest, cursor: Value) -> Step {
    Step::Request { request, cursor }
}
pub fn segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

pub fn provider_error(response: &HttpResponse) -> Option<Step> {
    if (200..300).contains(&response.status) {
        return None;
    }
    Some(match response.status {
        401 => error(
            "unauthorized",
            "Authentication expired or was revoked. Reconnect this account.",
        ),
        403 => error(
            "forbidden",
            "This connection does not have permission for the operation.",
        ),
        429 => error(
            "rate_limited",
            "Provider rate limit reached. Try again later.",
        ),
        404 => error(
            "not_found",
            "The resource was not found or is not accessible to this connection.",
        ),
        _ => error(
            "provider_error",
            &format!(
                "Provider returned HTTP {}. The operation was not confirmed.",
                response.status
            ),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn errors_do_not_echo_provider_payloads() {
        let result = provider_error(&HttpResponse {
            status: 400,
            body: "secret-token".into(),
            headers: BTreeMap::new(),
        })
        .unwrap();
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("secret-token")
        );
    }
    #[test]
    fn secret_debug_is_redacted() {
        let secret = SecretValue {
            fields: BTreeMap::from([("token".into(), "sensitive".into())]),
        };
        assert_eq!(format!("{secret:?}"), "SecretValue([redacted])");
    }
}
