use getkeys_core::{App, api::SecretValue, model::*};
use std::{collections::BTreeMap, path::PathBuf, process::Command};

struct Fixture(tempfile::TempDir);
impl Drop for Fixture {
    fn drop(&mut self) {
        let path = self.0.path().join("vault.db");
        keyring::Entry::new("dev.getkeys.vault", &path.to_string_lossy())
            .unwrap()
            .delete_credential()
            .unwrap();
    }
}

#[test]
fn cli_redacts_inventory_requires_deliberate_reveal_and_injects_only_into_the_child() {
    let fixture = Fixture(tempfile::tempdir().unwrap());
    let registry = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../registry/dist");
    let app = App::open(fixture.0.path().into(), registry.clone()).unwrap();
    app.vault
        .put(
            "credential",
            "test",
            &Credential {
                view: CredentialView {
                    id: "test".into(),
                    connection_id: "offline".into(),
                    provider_id: "test".into(),
                    name: "Test key".into(),
                    credential_type: "token".into(),
                    resource: "test".into(),
                    project: "app".into(),
                    environment: "test".into(),
                    created_at: None,
                    expires_at: None,
                    value_available: true,
                    can_rotate: false,
                    retired: false,
                    fields: Vec::new(),
                    resource_name: String::new(),
                    groups: Vec::new(),
                    problem: None,
                },
                details: BTreeMap::new(),
                value: Some(SecretValue {
                    fields: BTreeMap::from([("token".into(), "fixture-secret".into())]),
                }),
            },
        )
        .unwrap();
    app.save_binding(Binding {
        id: "binding".into(),
        credential_id: "test".into(),
        field: "token".into(),
        variable: "GETKEYS_CLI_TEST".into(),
        project: "app".into(),
        environment: "test".into(),
        destination: Destination::Local,
    })
    .unwrap();
    drop(app);
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_getkeys"))
            .arg("--data-dir")
            .arg(fixture.0.path())
            .arg("--registry")
            .arg(&registry)
            .args(args)
            .env_remove("GETKEYS_CLI_TEST")
            .output()
            .unwrap()
    };
    let inventory = cli(&["list"]);
    assert!(inventory.status.success());
    let inventory = String::from_utf8(inventory.stdout).unwrap();
    assert!(inventory.contains("Test key") && !inventory.contains("fixture-secret"));
    let denied = cli(&["get", "test"]);
    assert!(!denied.status.success() && denied.stdout.is_empty());
    let revealed = cli(&["get", "test", "--show"]);
    assert!(revealed.status.success());
    assert_eq!(
        String::from_utf8(revealed.stdout).unwrap().trim(),
        "fixture-secret"
    );
    let child = cli(&[
        "run",
        "--project",
        "app",
        "--environment",
        "test",
        "--",
        "node",
        "-e",
        "if(process.env.GETKEYS_CLI_TEST!=='fixture-secret')process.exit(1);console.log(process.argv[1]);process.exit(7)",
        "literal;echo unsafe",
    ]);
    assert_eq!(child.status.code(), Some(7));
    assert_eq!(
        String::from_utf8(child.stdout).unwrap().trim(),
        "literal;echo unsafe"
    );
}
