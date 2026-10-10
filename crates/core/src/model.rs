use getkeys_plugin_api::{Manifest, SecretValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PluginView {
    pub manifest: Manifest,
    pub installed: bool,
    pub update_available: bool,
    /// Browser sign-in works only when its OAuth registration is configured.
    pub browser_sign_in: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ConnectionView {
    pub id: String,
    pub plugin_id: String,
    pub name: String,
    pub state: ConnectionState,
    pub config: BTreeMap<String, String>,
    /// Local session it was imported from (see discover.rs), `token` when pasted, none for sign-in.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Connected,
    NeedsReauth,
    Disconnected,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Connection {
    pub view: ConnectionView,
    pub identity_id: String,
    pub auth: AuthMaterial,
    /// Every local session that resolved to this account, so none of them is offered again.
    #[serde(default)]
    pub linked: Vec<String>,
    /// Provider ID of the access token GetKeys created for itself, revoked when disconnecting.
    #[serde(default)]
    pub dedicated: Option<String>,
}

/// `ConnectionView::source` of an account that signs in with a token GetKeys created for itself.
pub const DEDICATED: &str = "getkeys";
/// `ConnectionView::source` of an account connected with a pasted token.
pub const PASTED: &str = "token";

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthMaterial {
    Bearer {
        token: String,
        refresh_token: Option<String>,
        expires_at: Option<i64>,
        broker_registration: Option<String>,
    },
    Aws {
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
        expires_at: Option<i64>,
        sso: Option<AwsSession>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AwsSession {
    pub region: String,
    pub access_token: String,
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: Option<String>,
    pub account_id: String,
    pub role_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CredentialView {
    pub id: String,
    pub connection_id: String,
    pub provider_id: String,
    pub name: String,
    pub credential_type: String,
    pub resource: String,
    pub project: String,
    pub environment: String,
    pub created_at: Option<String>,
    pub expires_at: Option<String>,
    pub value_available: bool,
    pub can_rotate: bool,
    pub retired: bool,
    /// Every field of the key in the plugin's display order. Secret values are never included.
    #[serde(default)]
    pub fields: Vec<KeyField>,
    /// Display name of `resource`, such as a repository or project name.
    #[serde(default)]
    pub resource_name: String,
    /// Groups the person filed this key under, which scope Export and Refresh.
    #[serde(default)]
    pub groups: Vec<String>,
    /// Why the key no longer works, such as being revoked or expired. None when it works or
    /// hasn't been checked.
    #[serde(default)]
    pub problem: Option<String>,
    /// The provider of a key held on its own, outside any account; empty otherwise.
    #[serde(default)]
    pub plugin_id: String,
}

/// One labelled part of a key, such as an access key ID, secret, region, or account ID.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct KeyField {
    pub key: String,
    pub label: String,
    /// Environment variable used by "Copy as .env"; empty when the field is not exported.
    pub env: String,
    pub secret: bool,
    /// The value itself for non-secret fields. Secret values are read through `reveal` or copy.
    pub value: Option<String>,
    /// Whether this device holds the value.
    pub available: bool,
}

/// The credential type of the key an account connected with.
pub const SIGN_IN: &str = "sign_in";

#[derive(Clone, Serialize, Deserialize)]
pub struct Credential {
    pub view: CredentialView,
    pub value: Option<SecretValue>,
    #[serde(default)]
    pub details: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Binding {
    pub id: String,
    pub credential_id: String,
    pub field: String,
    pub variable: String,
    pub project: String,
    pub environment: String,
    pub destination: Destination,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Destination {
    Local,
    Provider {
        connection_id: String,
        resource: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Rotation {
    pub id: String,
    pub credential_id: String,
    pub connection_id: String,
    pub resource: String,
    pub plugin_version: String,
    pub name: String,
    pub state: RotationState,
    pub candidate_id: Option<String>,
    pub bindings: Vec<Binding>,
    pub updated_bindings: Vec<String>,
    pub verified_bindings: Vec<String>,
    pub created_at: i64,
}
impl Rotation {
    pub fn is_active(&self) -> bool {
        !matches!(self.state, RotationState::Complete | RotationState::Aborted)
    }
    pub fn uses_connection(&self, id: &str) -> bool {
        self.connection_id == id || self.bindings.iter().any(|b| {
            matches!(&b.destination, Destination::Provider { connection_id, .. } if connection_id == id)
        })
    }
    pub fn uses_credential(&self, id: &str) -> bool {
        self.credential_id == id || self.candidate_id.as_deref() == Some(id)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RotationState {
    Planned,
    Creating,
    Uncertain { action: String },
    Stored,
    Distributing,
    AwaitingVerification,
    ReadyToRetire,
    Deactivating,
    Deactivated,
    Retiring,
    Complete,
    Aborted,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Activity {
    pub id: String,
    pub timestamp: i64,
    pub action: String,
    pub subject: String,
}

/// What deleting a key did.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Deletion {
    /// The provider revoked it, or it only existed here.
    Deleted,
    /// The provider can't revoke it through its API; its page was opened to do it there.
    Manual { url: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Snapshot {
    pub plugins: Vec<PluginView>,
    pub connections: Vec<ConnectionView>,
    pub credentials: Vec<CredentialView>,
    pub bindings: Vec<Binding>,
    pub rotations: Vec<Rotation>,
    pub activity: Vec<Activity>,
    /// Where the encrypted vault file lives.
    pub vault_path: String,
    /// Folders whose `.env` files are scanned for keys.
    #[serde(default)]
    pub project_folders: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthProgress {
    Browser {
        flow_id: String,
        url: String,
        user_code: Option<String>,
        interval: u64,
    },
    SelectRole {
        flow_id: String,
        roles: Vec<RoleChoice>,
    },
    Connected {
        connection: ConnectionView,
    },
    Pending {
        interval: u64,
    },
    /// A key found on this device was saved on its own.
    KeySaved {
        credential: Box<CredentialView>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RoleChoice {
    pub account_id: String,
    pub account_name: String,
    pub role: String,
}

/// One line of an exported `.env` file.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct EnvLine {
    pub credential_id: String,
    pub field: String,
    pub variable: String,
}

/// A field of a key that belongs to no provider.
#[derive(Clone, Deserialize, Serialize, TS)]
pub struct CustomField {
    /// Variable name the value is exported as, such as OPENAI_API_KEY.
    pub variable: String,
    pub value: String,
    pub secret: bool,
}

#[derive(Clone, Deserialize, Serialize, TS)]
pub struct ImportInput {
    pub connection_id: String,
    pub provider_id: Option<String>,
    pub resource: String,
    pub name: String,
    pub project: String,
    pub environment: String,
    pub fields: BTreeMap<String, String>,
}
