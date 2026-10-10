# GetKeys

A small desktop vault for provider credentials. GetKeys finds the provider sign-ins already on your machine, connects them in one click, and keeps your keys on your device.

**Windows first.** Tauri 2, React, TypeScript, and a shared Rust core. The CLI uses the same encrypted vault and rotation engine as the desktop app.

| Provider | Connect                                       | Retrieve                                | Create and replace                                 |
| -------- | --------------------------------------------- | --------------------------------------- | -------------------------------------------------- |
| GitHub   | GitHub CLI, `GH_TOKEN`, or a pasted token     | Import PAT values; repository inventory | New PATs made on its pre-filled page; API revoke   |
| AWS      | AWS CLI profiles, environment, or SSO         | IAM key metadata; capture new values    | Create → verify → deactivate → delete              |
| Vercel   | Vercel CLI, `VERCEL_TOKEN`, or a pasted token | Every access token; values for new ones | New tokens; token replacement                      |
| Supabase | Supabase CLI or a pasted access token         | Every project API key and its value     | New keys, in a new project if needed; key rotation |

OpenAI, Anthropic, Gemini, OpenRouter, Cloudflare, and GitLab connect with a management key and list (and, where the API allows, create, rotate, or revoke) their keys. Groq, Together AI, Mistral AI, DeepSeek, xAI, Cohere, Fireworks AI, Perplexity, Replicate, Hugging Face, Stripe, and Resend need no account: GetKeys finds their keys on this device and checks them. See [providers](docs/providers.md).

GetKeys is about keys. It creates, reads, updates, and deletes them without opening each provider's site, so they're ready to hand to an app or an AI agent. It doesn't track the apps that use them.

Providers do not expose the same capabilities. Existing AWS secret values and personal-token values cannot be recovered. Supabase legacy JWT keys are not automatically rotated.

## Run

Install Node 24, Rust 1.99, the `wasm32-unknown-unknown` target, Visual Studio C++ build tools, WebView2, and Strawberry Perl. The first native build compiles SQLCipher, OpenSSL, and Wasmtime and takes several minutes.

```powershell
npm ci
$env:PERL = 'C:\Strawberry\perl\bin\perl.exe'
npm run desktop -- dev
```

On macOS and Linux, skip Perl and the Windows tools; Linux also needs the [Tauri system packages](https://v2.tauri.app/start/prerequisites/#linux) and `libdbus-1-dev`. The vault key lives in the OS credential store: Keychain on macOS, a Secret Service such as GNOME Keyring or KWallet on Linux.

The signed provider packages are included in `registry/dist`. No setup is needed to connect: GetKeys reuses the CLI sign-ins on this machine or a token you paste. Browser sign-in is optional and needs your own app registrations; see [setup](docs/setup.md). `npm run dev` alone is the UI development server; vault operations require the native app.

## Use

1. In **Providers**, choose **Add**. GetKeys installs the signed provider package, signs in with every session for that provider on this device, and lists all the keys those accounts can see. A progress window shows each step; **Cancel** stops after the current one. If no local sign-in works, GetKeys asks for a token and opens the provider's token page.
2. Expand an added provider to manage its accounts: **Add account**, **Refresh**, or remove one. Each account shows the management key GetKeys uses to reach it. GetKeys starts with your CLI sign-in; **Create dedicated key** replaces it with a token GetKeys creates (Vercel), and **Rotate key** replaces that later. Where a provider can't create tokens, the provider's token page opens and you paste a new one. **Open**, on an added provider, goes to the page where its keys are made. **Remove provider** forgets its accounts and keys in GetKeys; nothing is deleted at the provider except management keys GetKeys created.
3. **Keys** lists every key from every provider: name, provider, project, and the masked value, with **Reveal**, **Copy**, **Rotate**, and **Delete** on each row. That includes keys found on this device, such as an AWS profile or a CLI sign-in; GetKeys reads those from where they live, so **Rotate** points you to the provider and **Delete** offers to remove the account instead. **Ready to use** is the default tab. Keys GetKeys found but can't use sit under **Others**, each with the reason: the provider shows its secret only once (such as existing Vercel tokens; **Fill in** adds the value), or the provider rejected it. Click anywhere on a row to expand it; the details say where the key came from (made by GetKeys, made on the provider's page, listed by an account, found on this device, or added by you).
4. Expand a row for its other fields (key ID, secret, token, region, URL…) with the environment variable each maps to, per-field copy, and **Copy .env** for all of them, and **Rotate**.
5. **Refresh** looks for new sign-ins on this device and new keys in all your accounts at once, then checks keys no provider listing confirms. Keys the provider rejects move to **Others** with the reason; a sign-in found on this device that the provider turns away is kept there too. Keys in accounts that may not list them stay ready to use, marked as not checked. Nothing about this is an error: the details go to `getkeys.log` next to the vault, not the screen.
6. **New key** asks one thing at a time: the provider (or **Any other service**, for a key given as named values; paste `NAME=value` lines to fill in several), then the account (picked by itself when there is one), then the project it belongs to, then a name. The provider makes the key through its API (OpenAI, Gemini, OpenRouter, Supabase, Vercel, AWS). Providers that make keys only on their own site (GitHub, Anthropic, GitLab) get their page opened, filled in; create the key there and copy it, and GetKeys picks it up from the clipboard, checks it, and keeps it. **Already have a key? Paste it** is there for keys made elsewhere. Each choice made folds into one line with **Change**. Where the provider can make projects (Supabase), **New project** makes one for the key right there and waits for it to start; its generated database password is saved as a key too. A key made while a group is open joins that group.
7. Groups sit next to **Ready to use** as tabs. **+** makes one and asks which keys go in it; **Choose keys** changes them later. A key's details list its groups; click one to open it, or use the small **+** to add the key to another group or a new one. Choosing a group narrows **Refresh** and **Export** to it. **Search** finds keys across everything; **Filter** narrows the tab in view by provider or status (**Not checked**, **Rotating**). **Export** writes the chosen keys (all, by default) to a `.env` file or the clipboard; a variable name used by two keys gets the project added.
8. **Delete** revokes the key with its provider. When the provider doesn't allow that, GetKeys opens its page; delete the key there, then choose **Done**.
9. **Rotate** a key from its details. GetKeys has the provider make a new key and retires the old one: the provider's own roll where it has one, otherwise a new key, then the old one revoked. The new key keeps the old one's name and groups. Providers that make keys only on their site get their page opened, and GetKeys picks up the new key and deletes the old one. A key added by hand is rotated by pasting its new values.

Revealed values hide after 30 seconds or loss of focus. Copied values clear after 30 seconds only if the clipboard still contains that value. Settings shows where the vault file lives. Export a passphrase-encrypted `.age` backup from Settings before moving devices. Restore uses an empty vault and never overwrites an existing one.

Closing the window keeps GetKeys in the tray (**Open**, **Quick copy…**, **Refresh keys**, **Quit**). **Ctrl+Shift+K** anywhere opens quick copy: type part of a key's name, then **Enter** copies its value. Settings turns on **Start with your computer** (starts in the tray). Links open the app: `getkeys://open`, `getkeys://key/<id>`, and `getkeys://add/<provider>`. The installer includes the `getkeys` command and, unless you clear the box on its first page, adds it to your user PATH; uninstalling removes it.

```powershell
getkeys ls                                    # REF, NAME, PROVIDER, GROUPS, STATUS; --json for scripts
getkeys run --group backend -- node server.js # keys as environment variables, no shell
getkeys env --group web -o .env               # or inject -i config.tpl -o config.yml
getkeys get getkeys://openai/prod --show
echo $value | getkeys add --name "Mailer" --custom MAILER_KEY --group backend
getkeys new supabase --new-project my-app --name "App server" --group web
```

`getkeys new <provider>` asks the provider for a new key, like **New key**. `--account` picks the account and `--project` the project (by name or ID) when there's more than one. `--new-project NAME` makes the project first (`--owner` picks the organisation, `--region americas|emea|apac` places a Supabase database) and waits for it to start. `--group` files the key.

Keys are named by references such as `getkeys://openai/prod` or `getkeys://custom/mailer/MAILER_KEY` (provider, key, optional field). `getkeys run` resolves references found in environment variables and `--env-file`, and passes values straight to the child process. `get` writes a value only with `--show`. The CLI goes through the running app (a named pipe, or a `0600` socket in the data folder) so the window updates, and opens the vault itself when the app isn't running. Also: `refresh`, `check`, `rotate`, `rm`, `providers ls|add|rm|open`, `backup`, `restore`, and `completions <shell>`.

`getkeys mcp` serves the keys to AI agents over the Model Context Protocol (`list_keys`, `get_secret`, `run_with_keys` with values redacted from output, `write_env_file`, `refresh`, `check_key`, `rotate_key`); each value an agent reads is logged. **Settings › Connect to an agent** turns it off, adds it to Claude Desktop, Claude Code, Cursor, and VS Code, or removes it again with **Disconnect** (the rest of the agent's configuration is kept). `getkeys native-host install` registers the host the GetKeys browser extension saves keys through.

The [browser extension](docs/extensions.md) (Chrome, Edge) notices a new key on a provider's page and offers to save it. The VS Code extension lists your keys, inserts `getkeys://` references, writes `.env` files, and runs tasks with keys. Both ship with each release.

## Validate

```powershell
npx playwright install chromium
npm run check
npm run desktop -- build --debug
npm run test:packaged
npm run test:live
```

`check` runs formatting, signature verification, generated types, production UI build, Clippy, the full Rust test suite, UI tests, native Windows tests (on Windows), browser accessibility checks, and npm audit. Provider bundle bytes are verified on Windows, where they are built and signed; on macOS the CLI journey tests are skipped because Keychain asks before a second unsigned program reads the vault key. CI runs the same stages (`node scripts/check.mjs lint|clippy|rust|web|native`) as parallel jobs: Clippy and the Rust tests on Windows, macOS, and Linux, the native journeys on Windows, and every package at once. Live checks **fail** when sandbox configuration is missing; they never silently skip. `npm run extensions:test` and `npm run extensions:build` cover the browser and VS Code extensions. See [live-test configuration](docs/live-tests.md).

CI gates releases. A `v*` tag runs CI, which checks and packages every platform in parallel, alongside the live provider checks when the repository variable `GETKEYS_LIVE_ENABLED` is `true`; once all pass it publishes, in one release: the Windows installer and `getkeys-windows-x64.exe`; macOS `.dmg` apps and `getkeys-macos-{arm64,x64}.tar.gz`; a Linux `.AppImage`, `.deb`, and `getkeys-linux-x64.tar.gz`; both extensions; and the signed marketplace. The CLI archives hold `getkeys` with the `registry/` it reads beside it. Running Release by hand (`workflow_dispatch`) is a dry run by default: it builds every package as workflow artifacts and publishes nothing. OS code signing is not configured yet: the Windows installer is unsigned and the macOS app is only ad-hoc signed, so macOS needs right-click > **Open** (or `xattr -dr com.apple.quarantine /Applications/GetKeys.app`) on first launch; marketplace signatures are separate from installer signing.

## Develop providers

Each provider has one manifest and a pure Wasm adapter under `plugins/`. The marketplace accepts only catalogs signed by the bundled trust root. Installed packages retain their signed catalog; artifacts are verified on load. Adding a provider does not require a new credential-management service or renderer adapter. A provider that only needs its keys found and checked is a manifest alone, with no Wasm. `cargo run -p xtask -- new-provider <id> [--declarative]` scaffolds one; see [providers](docs/providers.md).

For a development catalog, create your own signing key once, then rebuild:

```powershell
cargo run -p xtask -- init-signing
npm run plugins:build
npm run types
```

This intentionally replaces `registry/public-key.hex`. Keep `.local/registry-signing-key.hex` private. Releases of this repository must use the matching maintainer key via `GETKEYS_REGISTRY_SIGNING_KEY`; do not replace the production trust root just to publish an update.

See [architecture](docs/architecture.md) for the provider boundary and rotation recovery behavior.
