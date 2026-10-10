# GetKeys

A small desktop vault for provider credentials. GetKeys finds the provider sign-ins already on your machine, connects them in one click, and keeps your keys on your device.

**Windows first.** Tauri 2, React, TypeScript, and a shared Rust core. The CLI uses the same encrypted vault and rotation engine as the desktop app.

| Provider | Connect                                       | Retrieve                                | Replace                                                |
| -------- | --------------------------------------------- | --------------------------------------- | ------------------------------------------------------ |
| GitHub   | GitHub CLI, `GH_TOKEN`, or a pasted token     | Import PAT values; repository inventory | Guided PAT replacement; Actions secret delivery        |
| AWS      | AWS CLI profiles, environment, or SSO         | IAM key metadata; capture new values    | Create → verify → deactivate → delete                  |
| Vercel   | Vercel CLI, `VERCEL_TOKEN`, or a pasted token | Every access token; values for new ones | Guided token replacement; project environment delivery |
| Supabase | Supabase CLI or a pasted access token         | Every project API key and its value     | Modern server secret key rotation                      |

OpenAI, Anthropic, Gemini, OpenRouter, Cloudflare, and GitLab connect with a management key and list (and, where the API allows, create, rotate, or revoke) their keys. Groq, Together AI, Mistral AI, DeepSeek, xAI, Cohere, Fireworks AI, Perplexity, Replicate, Hugging Face, Stripe, and Resend need no account: GetKeys finds their keys on this device and checks them. See [providers](docs/providers.md).

Providers do not expose the same capabilities. Existing AWS secret values and personal-token values cannot be recovered. Supabase legacy JWT keys are not automatically rotated. Updating a Vercel environment variable requires a new deployment before consumer verification.

## Run

Install Node 24, Rust 1.99, the `wasm32-unknown-unknown` target, Visual Studio C++ build tools, WebView2, and Strawberry Perl. The first native build compiles SQLCipher, OpenSSL, and Wasmtime and takes several minutes.

```powershell
npm ci
$env:PERL = 'C:\Strawberry\perl\bin\perl.exe'
npm run desktop -- dev
```

The signed provider packages are included in `registry/dist`. No setup is needed to connect: GetKeys reuses the CLI sign-ins on this machine or a token you paste. Browser sign-in is optional and needs your own app registrations; see [setup](docs/setup.md). `npm run dev` alone is the UI development server; vault operations require the native app.

## Use

1. In **Providers**, choose **Add**. GetKeys installs the signed provider package, signs in with every session for that provider on this device, and lists all the keys those accounts can see. A progress window shows each step; **Cancel** stops after the current one. If no local sign-in works, GetKeys asks for a token and opens the provider's token page.
2. Expand an added provider to manage its accounts: **Add account**, **Refresh**, or remove one. Each account shows the management key GetKeys uses to reach it. GetKeys starts with your CLI sign-in; **Create dedicated key** replaces it with a token GetKeys creates (Vercel), and **Rotate key** replaces that later. Where a provider can't create tokens, the provider's token page opens and you paste a new one. **Open** goes to the provider's page where its keys are made. **Remove provider** forgets its accounts and keys in GetKeys; nothing is deleted at the provider except management keys GetKeys created.
3. **Keys** lists every key from every provider: name, provider, project, and the masked value, with **Reveal**, **Copy**, **Rotate**, and **Delete** on each row. That includes keys found on this device, such as an AWS profile or a CLI sign-in; GetKeys reads those from where they live, so **Rotate** points you to the provider and **Delete** offers to remove the account instead. **Ready to use** is the default tab. Keys a provider lists without a value (such as existing Vercel tokens) sit under **Empty profiles**; **Fill in** adds the value. Click anywhere on a row to expand it.
4. Expand a row for its other fields (key ID, secret, token, region, URL…) with the environment variable each maps to, per-field copy, and **Copy .env** for all of them.
5. **Refresh** looks for new sign-ins on this device and new keys in all your accounts at once, then checks keys no provider listing confirms; keys the provider rejects are marked **Invalid**. Rejected keys, and keys in accounts that lack permission to list them, go in the **Failed** group instead of failing the refresh; they leave it once a refresh gets through. **New key** adds one by hand: pick the provider and project, or **None** for a key from any other service, given as named values (paste `NAME=value` lines to fill in several).
6. Groups sit next to **Ready to use** as tabs. **+** makes one and asks which keys go in it; **Choose keys** changes them later, or file a key from its details. Choosing a group narrows **Refresh** and **Export** to it. **Export** writes the chosen keys (all, by default) to a `.env` file or the clipboard; a variable name used by two keys gets the project added.
7. **Delete** revokes the key with its provider. When the provider doesn't allow that, GetKeys opens its page; delete the key there, then choose **Done**.
8. Register the apps that read a key: a local process, a GitHub repository, or a Vercel project.
9. **Rotate** a key from its row. Progress stays on the key: verify each consumer after deployment, then retire the old key. A key added by hand is rotated by pasting its new values.

Revealed values hide after 30 seconds or loss of focus. Copied values clear after 30 seconds only if the clipboard still contains that value. Settings shows where the vault file lives. **Project folders** in Settings makes Refresh also read the `.env` files of the projects in those folders and list the provider keys it finds there. Export a passphrase-encrypted `.age` backup from Settings before moving devices. Restore uses an empty vault and never overwrites an existing one.

Closing the window keeps GetKeys in the tray (**Open**, **Quick copy…**, **Refresh keys**, **Quit**). **Ctrl+Shift+K** anywhere opens quick copy: type part of a key's name, then **Enter** copies its value. Settings turns on **Start with your computer** (starts in the tray). Links open the app: `getkeys://open`, `getkeys://key/<id>`, and `getkeys://add/<provider>`. The installer includes the `getkeys` command and, unless you clear the box on its first page, adds it to your user PATH; uninstalling removes it.

```powershell
getkeys ls                                    # REF, NAME, PROVIDER, GROUPS, STATUS; --json for scripts
getkeys run --group backend -- node server.js # keys as environment variables, no shell
getkeys env --group web -o .env               # or inject -i config.tpl -o config.yml
getkeys get getkeys://openai/prod --show
echo $value | getkeys add --name "Mailer" --custom MAILER_KEY --group backend
```

Keys are named by references such as `getkeys://openai/prod` or `getkeys://custom/mailer/MAILER_KEY` (provider, key, optional field). `getkeys run` resolves references found in environment variables and `--env-file`, and passes values straight to the child process. `get` writes a value only with `--show`. The CLI goes through the running app (a named pipe, or a `0600` socket in the data folder) so the window updates, and opens the vault itself when the app isn't running. Also: `refresh`, `check`, `rotate`, `rm`, `providers ls|add|rm|open`, `backup`, `restore`, and `completions <shell>`.

`getkeys mcp` serves the keys to AI agents over the Model Context Protocol (`list_keys`, `get_secret`, `run_with_keys` with values redacted from output, `write_env_file`, `refresh`, `check_key`, `rotate_key`); each value an agent reads is logged. **Settings › Connect to an agent** turns it off or adds it to Claude Desktop, Claude Code, Cursor, and VS Code. `getkeys native-host install` registers the host the GetKeys browser extension saves keys through.

The [browser extension](docs/extensions.md) (Chrome, Edge) notices a new key on a provider's page and offers to save it. The VS Code extension lists your keys, inserts `getkeys://` references, writes `.env` files, and runs tasks with keys. Both ship with each release.

## Validate

```powershell
npx playwright install chromium
npm run check
npm run desktop -- build --debug
npm run test:packaged
npm run test:live
```

`check` runs formatting, signature verification, generated types, production UI build, Clippy, the full Rust test suite, UI tests, native Windows tests, browser accessibility checks, and npm audit. Live checks **fail** when sandbox configuration is missing; they never silently skip. `npm run extensions:test` and `npm run extensions:build` cover the browser and VS Code extensions. See [live-test configuration](docs/live-tests.md).

CI gates releases. A `v*` tag runs CI, then the live provider checks when the repository variable `GETKEYS_LIVE_ENABLED` is `true`, then publishes the installer, the `getkeys` CLI, both extensions, and the signed marketplace. Signing the Windows installer with an OS code-signing certificate is not configured yet; marketplace signatures are separate from installer signing.

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
