pub mod app;
pub mod auth;
pub mod commands;
pub mod discover;
pub mod http;
pub mod model;
pub mod plugins;
pub mod rotation;
pub mod vault;
pub use app::App;
pub use getkeys_plugin_api as api;

pub fn new_id() -> anyhow::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Operating system randomness is unavailable"))?;
    Ok(hex::encode(bytes))
}
pub fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}
