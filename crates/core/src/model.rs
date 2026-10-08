use getkeys_plugin_api::{Manifest, SecretValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PluginView {
    pub manifest: Manifest,
    pub installed: bool,
    pub update_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ConnectionView {
    pub id: String,
    pub plugin_id: String,
    pub name: String,
    pub state: ConnectionState,
    pub config: BTreeMap<String, String>,
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
}

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
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Credential {
    pub view: CredentialView,
    pub value: Option<SecretValue>,
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

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Snapshot {
    pub plugins: Vec<PluginView>,
    pub connections: Vec<ConnectionView>,
    pub credentials: Vec<CredentialView>,
    pub bindings: Vec<Binding>,
    pub rotations: Vec<Rotation>,
    pub activity: Vec<Activity>,
    pub setup: Vec<String>,
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
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RoleChoice {
    pub account_id: String,
    pub account_name: String,
    pub role: String,
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
