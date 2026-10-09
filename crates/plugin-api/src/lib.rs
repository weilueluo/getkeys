use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub description: String,
    pub homepage: String,
    /// Managed providers ship code that lists and manages keys remotely; declarative ones are
    /// just this manifest: where their keys live on a device and how to check one.
    #[serde(default)]
    pub kind: ProviderKind,
    /// Groups providers in Add provider, such as `ai`, `cloud`, `database` or `devtools`.
    #[serde(default)]
    pub category: String,
    pub hosts: Vec<String>,
    #[serde(default)]
    pub auth: AuthMethod,
    #[serde(default)]
    pub operations: Vec<OperationKind>,
    #[serde(default)]
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
    /// Regular expression the key's first secret field matches, used to recognise pasted keys
    /// and keys found in `.env` files.
    #[serde(default)]
    pub key_pattern: String,
    /// How to ask the provider whether one of its keys still works.
    #[serde(default)]
    pub validate: Option<Validate>,
    /// Where this provider's keys and sign-ins live on a device.
    #[serde(default)]
    pub sources: Vec<Source>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    Managed,
    Declarative,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthMethod {
    /// Connect by pasting a management key or token.
    #[default]
    Token,
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

/// A request that succeeds only with a working key. 401 and 403 mean the key is rejected.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(deny_unknown_fields)]
pub struct Validate {
    pub url: String,
    /// Header carrying the key.
    #[serde(default = "authorization")]
    pub header: String,
    /// Put before the key in the header.
    #[serde(default = "bearer")]
    pub prefix: String,
    /// Key field sent.
    #[serde(default = "token")]
    pub field: String,
    /// Other fixed headers, such as an API version.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

fn authorization() -> String {
    "Authorization".into()
}
fn bearer() -> String {
    "Bearer ".into()
}
fn token() -> String {
    "token".into()
}

/// A place on this device that may hold a key or a sign-in.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
pub struct Source {
    /// Stable ID, unique across providers. Environment variables use `env:NAME`.
    #[serde(default)]
    pub id: String,
    /// Where it was found, such as "Hugging Face CLI".
    #[serde(default)]
    pub label: String,
    /// Whether it signs an account in, or is a key to list on its own.
    #[serde(default)]
    pub role: SourceRole,
    /// Key field the found value fills.
    #[serde(default = "token")]
    pub field: String,
    #[serde(flatten)]
    pub from: SourceKind,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum SourceRole {
    #[default]
    Account,
    Key,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceKind {
    /// An environment variable.
    Env { var: String },
    /// The newest existing file of `paths`. Paths may start with `~`, `{config}`, `{data}`,
    /// `{local_data}` or `{env:NAME}`. Text files hold the value; JSON, TOML and INI files
    /// hold it at `pointer` (a JSON pointer, or `section.key` for INI).
    File {
        paths: Vec<String>,
        #[serde(default)]
        format: FileFormat,
        #[serde(default)]
        pointer: String,
        /// Pointer to an expiry, in seconds or milliseconds since the epoch.
        #[serde(default)]
        expires_pointer: String,
        /// Command that renews an expiring session before reading again.
        #[serde(default)]
        renew: Vec<String>,
    },
    /// A command's standard output, run without a shell.
    Command { argv: Vec<String> },
    /// An OS credential store entry.
    Keyring {
        service: String,
        user: String,
        /// Windows credential target name, when it differs.
        #[serde(default)]
        target: String,
        /// The value is stored by go-keyring.
        #[serde(default)]
        go_keyring: bool,
        /// File or folder that hints the entry exists, so other systems aren't asked to unlock
        /// their keychain while scanning.
        #[serde(default)]
        hint: String,
    },
    /// Every account signed in to the GitHub CLI.
    GhCli,
    /// `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`.
    AwsEnv,
    /// Access key profiles in the AWS shared credentials file.
    AwsProfiles,
    /// IAM Identity Center profiles in the AWS config file.
    AwsSso,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum FileFormat {
    #[default]
    Text,
    Json,
    Toml,
    Ini,
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
    /// Replaces a key's secret in place; the old value stops working.
    RotateCredential,
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
    RotateCredential {
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
            Self::RotateCredential { .. } => OperationKind::RotateCredential,
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
                | Self::RotateCredential { .. }
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
