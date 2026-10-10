# Adding a provider

A provider teaches GetKeys about one service's keys. Every provider is a folder in `plugins/<id>/`
with a `manifest.json`. There are two kinds:

- **Declarative** providers are only a manifest. They say where the service's keys live on a device
  (environment variables, CLI config files) and how to ask the service whether a key still works.
  Use this kind when the service has no API for managing keys, or when listing keys is all you need
  from a paste or a scan. Examples: `plugins/groq`, `plugins/huggingface`, `plugins/stripe`.
- **Managed** providers also ship a small Rust plugin compiled to WebAssembly. GetKeys connects an
  account with a management token and the plugin lists, creates, rotates and revokes keys through the
  service's API. Examples: `plugins/openai`, `plugins/vercel`, `plugins/gitlab`.

## Scaffold

```powershell
cargo run -p xtask -- new-provider acme --declarative   # plugins/acme/manifest.json
cargo run -p xtask -- new-provider acme                 # plus Cargo.toml, src/lib.rs and a workspace member
```

The scaffold fills in placeholders (`api.acme.com`, `ACME_API_KEY`); replace them with the real
values from the service's API docs before packaging.

## Manifest fields

Types are defined in `crates/plugin-api/src/lib.rs` (`Manifest`).

| Field                 | Notes                                                                                                                                                                                                                                                                                                                                                                                   |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`                  | `official.<id>` for first-party providers. Must be unique.                                                                                                                                                                                                                                                                                                                              |
| `name`                | Display name. `ProviderMark` in `apps/desktop/src/ui.tsx` looks up a logo by `name.toLowerCase()`.                                                                                                                                                                                                                                                                                      |
| `kind`                | `declarative` or `managed` (the default). Declarative providers cannot declare `operations`.                                                                                                                                                                                                                                                                                            |
| `category`            | `ai`, `cloud`, `database`, `devtools`, `payments`, `email`… Groups providers in Add provider.                                                                                                                                                                                                                                                                                           |
| `hosts`               | Every host the provider talks to, lowercase, optionally `*.`-prefixed. Requests to other hosts are refused.                                                                                                                                                                                                                                                                             |
| `credential_fields`   | Fields of one key. The first secret field is the key itself; give it an `env` name used when exporting.                                                                                                                                                                                                                                                                                 |
| `account_fields`      | Managed only: fields of the management token pasted to connect an account.                                                                                                                                                                                                                                                                                                              |
| `token_page`          | Where the user creates the token or key to paste.                                                                                                                                                                                                                                                                                                                                       |
| `key_page`            | For providers whose API can't make keys: the page that makes one, as a template with `{name}` (the key's name) and `{owner}` (the resource's owner, or the account). GetKeys opens it filled in and picks the new key up from the clipboard. Defaults to `token_page`.                                                                                                                  |
| `revoke_by_value`     | Managed only: `revoke_credential` takes the key's value rather than its ID (inject it with `json_array`). Lets GetKeys delete keys it can't look up by ID.                                                                                                                                                                                                                              |
| `key_pattern`         | Regex matching the key's value. Used to recognise pasted keys and keys in `.env` files, so keep it specific enough not to match other providers' keys.                                                                                                                                                                                                                                  |
| `validate`            | `{ url, header?, prefix?, field?, headers? }`. A GET that succeeds only with a working key; HTTP 401 or 403 marks the key rejected. Defaults to `Authorization: Bearer <key>`. Host must be in `hosts`.                                                                                                                                                                                 |
| `sources`             | Where keys and sign-ins live on a device (below).                                                                                                                                                                                                                                                                                                                                       |
| `auth`                | Managed only: how an account connects. `token` (paste a management key) is the default; `device`, `broker` and `aws_sso` are for OAuth-style flows.                                                                                                                                                                                                                                     |
| `operations`          | Managed only: what the plugin implements (below).                                                                                                                                                                                                                                                                                                                                       |
| `account_credentials` | Managed only: the account's own tokens are listed under resource `""`, and the one in use is listed with credential type `sign_in`.                                                                                                                                                                                                                                                     |
| `new_resource`        | Managed only, with `create_resource`: `{ noun, options?, generated? }`. `noun` names what is made (`repository`, `project`); each option is a choice list with a `default`; `generated` (a field with `key`, `label` and `env`) has GetKeys make up a secret such as a database password, save it as a key first, and put it in the request body field `key`; the plugin never sees it. |

For a header other than `Authorization`, set `header` and an empty `prefix`, for example
`{ "url": "https://api.anthropic.com/v1/models", "header": "x-api-key", "prefix": "", "headers": { "anthropic-version": "2023-06-01" } }`.

## Sources

Each source fills the key field `field` (default `token`). `role` is `account` (a management token
that connects an account; the default) or `key` (a key listed on its own). Source IDs must be unique
across all providers: environment variables are identified as `env:NAME`, so two providers cannot
both claim the same variable; other kinds need an explicit `id`.

```json
{ "kind": "env", "var": "GROQ_API_KEY", "role": "key" }
{ "id": "huggingface", "label": "Hugging Face CLI", "kind": "file", "role": "key",
  "paths": ["{env:HF_HOME}/token", "~/.cache/huggingface/token"] }
{ "id": "firectl", "label": "Fireworks CLI", "kind": "file", "role": "key",
  "paths": ["~/.fireworks/auth.ini"], "format": "ini", "pointer": "api_key" }
{ "id": "gcloud", "label": "Google Cloud CLI", "kind": "command", "argv": ["gcloud", "auth", "print-access-token"] }
```

- `file`: the newest existing path wins. Paths may start with `~/`, `{home}`, `{config}`, `{data}`,
  `{local_data}` or `{env:NAME}`, and the last segment may contain `*`. `format` is `text` (the
  default; the whole file), `json` or `toml` (with a JSON pointer such as `/default/api_key`) or `ini`
  (`section.key`, or just `key` for files without sections). `expires_pointer` and `renew` handle
  CLI sessions that expire.
- `command`: standard output of a command run without a shell. It runs on every scan, so prefer files.
- `keyring`: an OS credential store entry. Set `hint` to a file that exists only when the entry might.
- `gh_cli`, `aws_env`, `aws_profiles`, `aws_sso`: built-in readers for the GitHub and AWS CLIs.

YAML files are not supported.

## Managed plugins

A managed plugin is a `cdylib` crate named `getkeys-plugin-<id>` that exports `describe` (returns the
manifest) and `run`. `run` receives a `Call` and returns a `Step`: either an HTTP request for GetKeys
to send, an outcome, or an error. Multi-request flows (pagination, look-up then delete) return a
request with a `cursor`; GetKeys calls `run` again with the response and that cursor. The plugin never
sees secrets and has no clock or network of its own.

Operations, each declared in `operations`:

| Operation               | Returns                   | Notes                                                                                                                                                                                                                                                                       |
| ----------------------- | ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `identity`              | `Identity { id, name }`   | The connected account. Used to avoid connecting the same account twice.                                                                                                                                                                                                     |
| `list_resources`        | `Resources`               | Projects, workspaces… Keys are listed per resource. Pseudo-resources (such as OpenAI's `admin`) are fine.                                                                                                                                                                   |
| `list_credentials`      | `Credentials`             | Keys of a resource. Leave `value` empty when the API never returns it; put hints in `details`.                                                                                                                                                                              |
| `read_credential`       | `Credential`              | Fetches the value of a listed key on demand.                                                                                                                                                                                                                                |
| `create_credential`     | `Credential` with `value` | Fail with `value_unavailable` if the API did not return the new value.                                                                                                                                                                                                      |
| `validate_credential`   | `Done` or `Identity`      | Send the request with `auth = RequestAuth::Candidate` to check the key being imported or checked.                                                                                                                                                                           |
| `deactivate_credential` | `Done`                    | Disables a key without deleting it; rotation uses it before revoking.                                                                                                                                                                                                       |
| `revoke_credential`     | `Done`                    | Deletes the key.                                                                                                                                                                                                                                                            |
| `rotate_credential`     | `Credential` with `value` | The service's native roll/rotate. The result replaces the old key (same `id` means rolled in place).                                                                                                                                                                        |
| `write_secret`          | `Done`                    | Legacy: older manifests may list it; GetKeys no longer calls it.                                                                                                                                                                                                            |
| `list_owners`           | `Owners`                  | Accounts and organisations a new resource can belong to. Put the personal account first.                                                                                                                                                                                    |
| `create_resource`       | `Resource`                | Makes a repository or project from `owner`, `name` and `options`. Set `pending` while it is still starting, `url` to open it on the provider's site, and `note` for anything the user should know (such as a link that failed). Use `name_error` to explain rejected names. |
| `guide`                 | `Guide { url, message }`  | Manual steps when the API cannot do something.                                                                                                                                                                                                                              |

Set `can_rotate` on a listed key only when GetKeys can replace it without losing anything: it needs
`create_credential` and `revoke_credential`, and the new key must have the same access as the old one.

Requests carry the connection's token as `Authorization: Bearer` unless the plugin sets the internal
header `x-getkeys-auth-header` to an allowed header name (see `crates/core/src/http.rs`). Plugins
cannot set `Authorization`, `Cookie` or `Host`, and only the `link` and `retry-after` response headers
reach them.

Test plugins by feeding recorded responses to `handle(Call { response: Some(...), .. })` and asserting
on the returned `Step`; see the tests at the bottom of any plugin. Tests must not hit the network.

## Logos

Add the provider's [Simple Icons](https://simpleicons.org) path (CC0, 24×24 viewBox) to `logos` in
`apps/desktop/src/ui.tsx`, keyed by the lowercased provider name. Providers without one show a
monogram.

## Build, sign and verify

```powershell
cargo test -p getkeys-plugin-<id>              # managed plugins
npx prettier --write plugins/*/manifest.json
npm run plugins:build                          # compiles plugins to wasm, validates, packages and signs registry/dist
node scripts/verify-plugins.mjs                # signed manifests match the sources
cargo run -q -p xtask -- verify                # signature and artifact digests
```

`plugins:build` validates every manifest (hosts, key pattern, `validate` URL, unique sources, no
operations on declarative providers) and signs `registry/dist/index.json` with
`.local/registry-signing-key.hex` (or `GETKEYS_REGISTRY_SIGNING_KEY`). Create a development key with
`cargo run -p xtask -- init-signing`. A declarative package's digest covers its manifest; a managed
one's covers its wasm. The build uses `--locked`, so after adding a managed provider run
`cargo test -p getkeys-plugin-<id>` (or `cargo check`) once to record it in `Cargo.lock`.
