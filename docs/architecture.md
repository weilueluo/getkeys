# Architecture

The repository follows the explicit contracts, cohesive modules, and small deterministic functions in `../common/code.md`. The provider manifest is the source of truth for permissions, connection fields, credential fields, and UI capabilities. Rust types generate the renderer's TypeScript command and metadata contract.

```mermaid
flowchart LR
  UI[React desktop UI] --> IPC[Tauri typed commands]
  CLI[CLI] --> Core[Rust core]
  IPC --> Core
  Core --> Vault[SQLCipher vault]
  Vault --- OS[OS credential store]
  Core --> Runtime[Extism / Wasmtime]
  Catalog[Signed marketplace] --> Runtime
  Runtime --> Step[Request or result]
  Step --> HTTP[Host HTTP and signing]
  HTTP --> Providers[Provider APIs]
  Core --> Browser[System browser]
  Browser --> OAuth[Provider authorization]
  Core --> Broker[OAuth broker]
  Broker --> OAuth
```

| Directory | Responsibility |
| --- | --- |
| `crates/plugin-api` | Versioned provider protocol |
| `crates/core` | Vault, authorization, plugin runtime, HTTP, rotation |
| `plugins/*` | Provider manifests and pure request/response adapters |
| `apps/desktop` | Minimal React interface and native IPC |
| `apps/cli` | Metadata, auth, typed commands, process injection, backups |
| `services/auth-broker` | Confidential OAuth exchanges for Supabase and Vercel |
| `tools/xtask` | Signing, packaging, type generation, live checks |

## Provider boundary

A provider package exports `describe()` and `run(Call)`. It receives an operation, non-secret configuration, the previous response, and its continuation cursor. It returns an HTTP request, a result, or a safe error. The host owns bearer authentication, AWS SigV4, GitHub sealed-box encryption, and consumer-value injection. Authentication tokens are not passed to guest code. Credential values returned by provider APIs necessarily pass through the adapter that parses them.

Guests have no WASI, filesystem, environment, direct HTTP imports, or arbitrary host functions. The host restricts HTTPS requests to declared DNS hosts and refuses redirects. Each guest has a 64 MiB memory limit and a five-second execution limit; an operation permits at most 100 HTTP steps. Signed catalog entries bind the manifest and SHA-256 artifact digest. These boundaries protect against changed artifacts and undeclared access, not a malicious approved publisher granted access to its declared provider domains.

## Keys and fields

The manifest describes every key with `credential_fields`, and the key an account signs in with using `account_fields`. Each field has a label, a `secret` flag (default `true`), and an optional `env` variable name. Plugins can return non-secret `details` with a credential, such as an AWS access key ID or a Supabase project URL. The core turns the schema, stored value, and details into one `KeyField` list per key. The renderer receives non-secret values and only the availability of secret values. That gives every provider the same table, per-field copy, explicit reveal, and **Copy .env**, which writes `ENV=value` lines for exported fields to the clipboard for 30 seconds.

Connecting an account stores its own credential as a management key (`sign_in`). The renderer lists it with the account under Providers. When it was found on this device (a CLI session, AWS profile, or environment variable), it is also one of the person's keys, so it is listed under Keys too; it changes at its source, and removing it removes the account. Switching such an account to a pasted or dedicated key keeps the local key as an ordinary key. The snapshot describes each installed plugin by its installed manifest, so the renderer asks only for operations the running code supports until an update is approved. `scan_sources` connects each local session not seen before (for one plugin when it is added, or for every added plugin on refresh). The renderer then calls `sync` for each account and place: the account itself for account-level plugins (`account_credentials`), otherwise each resource. It runs these as a cancellable job that stops between calls. `sync` records each key's resource name and forgets keys the provider no longer lists. Accounts the user disconnected are never reconnected by a refresh. `dedicate` creates and validates a token named `GetKeys` with the current management key, then switches the account to it. A borrowed CLI token stays usable elsewhere, so it is kept as an ordinary key; a previously dedicated token is revoked. `disconnect` revokes the dedicated token too. Plugins without token creation replace the management key through `connect_token`, and a rotation never retires one. `delete` revokes a key through `revoke_credential` when the plugin supports it; otherwise it returns the provider's page, and `confirm_deleted` removes the key after the user deletes it there. At startup, GetKeys installs catalog updates that don't widen a plugin's permissions; broader updates still need approval. `uninstall` disconnects each account of the provider, forgets its keys, consumers, and connection records, then removes the package.

A refresh runs local connection, listing, and checking for every provider and account concurrently, bounded per stage. The native host serves those slow, network-bound commands (`sync`, `resources`, `connect_local`, `scan_sources`, `check`) on a fork of the app: a second connection to the same vault, opened under the shared lock and used after it is released. The vault runs in WAL mode and every write takes an `IMMEDIATE` transaction, so concurrent writers queue on the database lock instead of failing. Renewing an expiring management key keeps that transaction open, so two forks never spend a rotating refresh token twice. After listing, `check` validates each key no provider listing confirmed (imported keys, and keys of providers that can't list); a `401` or `403` stores a problem, which the snapshot reports as `problem` alongside expiry and sign-in failures. The renderer marks those keys **Invalid**.

Keys added by hand without a provider (`save_custom`) have the credential type `custom` and no connection. Their fields are the variable names the person chose; secret values go to the encrypted value and the rest are stored as details. Every key carries `groups`, set with `set_groups` and kept across syncs and rotations. The renderer scopes Refresh to the accounts behind a group's keys and Export to its keys. Groups made in the renderer before they have keys are remembered in local storage. After a refresh, the renderer puts keys whose listing returned `forbidden`, or that `check` marked with a problem, in the `Failed` group, and takes keys out of it once their account lists them again. `export_env` is a native command, not a vault request: the renderer sends credential IDs, field names, and variable names, and the host writes the values to a file picked in the system save dialog, or to the clipboard for 60 seconds. Values never pass through the renderer.
## Vault and frontend

SQLCipher encrypts the database and journals. A random 256-bit vault key is stored through the OS credential store. Startup fails if the key is missing for an existing vault. The renderer receives metadata until an explicit reveal. Browser builds have no vault fallback, local-storage credential store, or bundled sample accounts.

Backups use age passphrase encryption. Restoring into a populated vault fails. The audit trail records action names and record IDs, never secret payloads. Same-user malware, screen capture, clipboard history, and secrets deliberately inherited by child processes are outside database encryption's protection.

## Rotation

```mermaid
stateDiagram-v2
  Planned --> Creating
  Creating --> Stored: value persisted
  Creating --> Planned: no provider write occurred
  Creating --> Uncertain: write outcome unknown
  Uncertain --> Stored: reconcile replacement
  Stored --> Distributing: candidate validates
  Distributing --> AwaitingVerification: consumers updated
  AwaitingVerification --> ReadyToRetire: user verifies every consumer
  ReadyToRetire --> Deactivated: AWS deactivation
  Deactivated --> Complete: delete old key
  ReadyToRetire --> Complete: revoke old key
```

A unique active rotation per account/resource prevents two AWS workflows racing for the second key slot. State transitions compare the persisted state before mutating a provider. A lost creation response is never retried automatically. Reconcile using the provider's replacement ID; one-time values must be imported if the response was lost. Idempotent destination writes resume after interruption. A successful write is not deployment verification.

The old key remains until verification and a separate retirement action. AWS deactivation and deletion are separate steps. Unknown retirement outcomes use inventory reconciliation before retrying. Plugin changes and affected consumer edits are blocked while a rotation is active. With no registered consumers, the UI explicitly requires the operator to update and test untracked consumers before retiring the old key.

Guided GitHub/Vercel replacement uses the same distribution and consumer-verification stages. Create and import a replacement with the required scopes, select it from the old key's replacement dialog, then update and verify consumers. Revoke the old token in the provider dashboard and explicitly confirm that action in GetKeys. The audit event records this as manual confirmation. Automated lifecycle operations are exposed only for credentials whose scope can be preserved.
