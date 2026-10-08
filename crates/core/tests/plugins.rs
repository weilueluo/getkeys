use getkeys_core::{
    api::*,
    plugins::{Package, Registry, Runtime},
};
use std::{collections::BTreeMap, path::PathBuf};

fn registry(installed: PathBuf) -> Registry {
    Registry::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../registry/dist"),
        installed,
    )
    .expect("Build signed plugins with npm run plugins:build before running the complete suite")
}
#[test]
fn all_real_wasm_packages_install_and_describe_their_signed_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let registry = registry(dir.path().into());
    assert_eq!(registry.catalog.packages.len(), 4);
    for p in &registry.catalog.packages {
        registry.install(&p.manifest.id, false).unwrap();
        assert_eq!(
            registry
                .installed_package(&p.manifest.id)
                .unwrap()
                .unwrap()
                .manifest,
            p.manifest
        );
    }
}
#[test]
fn github_wasm_uses_host_transport_and_cannot_escalate_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    let registry = registry(dir.path().into());
    registry.install("official.github", false).unwrap();
    let mut plugin = registry.load("official.github").unwrap();
    let result = plugin
        .execute(Operation::Identity, BTreeMap::new(), |request| {
            assert_eq!(request.url, "https://api.github.com/user");
            Ok(HttpResponse {
                status: 200,
                body: r#"{"id":123,"login":"sandbox-user"}"#.into(),
                headers: BTreeMap::new(),
            })
        })
        .unwrap();
    assert!(matches!(result,Outcome::Identity(Identity { name,.. }) if name=="sandbox-user"));
    assert!(
        plugin
            .execute(
                Operation::CreateCredential {
                    resource: "owner/repo".into(),
                    name: "bad".into()
                },
                BTreeMap::new(),
                |_| panic!("Unsupported operation reached the network")
            )
            .is_err()
    );
}
#[test]
fn tampered_installed_code_never_executes() {
    let dir = tempfile::tempdir().unwrap();
    let registry = registry(dir.path().into());
    registry.install("official.github", false).unwrap();
    let p = registry
        .installed_package("official.github")
        .unwrap()
        .unwrap();
    std::fs::write(dir.path().join(format!("{}.wasm", p.sha256)), b"corrupt").unwrap();
    assert!(registry.load("official.github").is_err());
}
fn package() -> Package {
    let dir = tempfile::tempdir().unwrap();
    registry(dir.path().into())
        .package("official.github")
        .unwrap()
        .clone()
}
#[test]
fn sandbox_has_no_wasi_filesystem() {
    let bytes=wat::parse_str(r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32))) (func (export "run") (result i32) i32.const 0))"#).unwrap();
    assert!(Runtime::new(package(), &bytes).is_err());
}
#[test]
fn runaway_plugin_is_interrupted() {
    let bytes =
        wat::parse_str(r#"(module (func (export "run") (result i32) (loop br 0) i32.const 0))"#)
            .unwrap();
    let mut plugin = Runtime::new(package(), &bytes).unwrap();
    let start = std::time::Instant::now();
    assert!(
        plugin
            .execute(Operation::Identity, BTreeMap::new(), |_| panic!(
                "No network allowed"
            ))
            .is_err()
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(15));
}
