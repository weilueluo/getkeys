# GetKeys

A small desktop vault for provider credentials. Browse a plugin, add it, and authorize your account in the browser. Your keys stay on your device.

**Windows first.** Tauri 2, React, TypeScript, and a shared Rust core. The CLI uses the same encrypted vault and rotation engine as the desktop app.

| Plugin | Connect | Retrieve | Replace |
| --- | --- | --- | --- |
| GitHub | Device authorization | Import PAT values; repository inventory | Guided PAT replacement; Actions secret delivery |
| AWS | IAM Identity Center | IAM key metadata; capture new values | Create → verify → deactivate → delete |
| Vercel | Integration OAuth | Import token values; project inventory | Guided token replacement; project environment delivery |
| Supabase | Management OAuth | Project key inventory and explicit reveal | Modern server secret key rotation |

Providers do not expose the same capabilities. Existing AWS secret values and personal-token values cannot be recovered. Supabase legacy JWT keys are not automatically rotated. Updating a Vercel environment variable requires a new deployment before consumer verification.

## Run

Install Node 24, Rust 1.99, the `wasm32-unknown-unknown` target, Visual Studio C++ build tools, WebView2, and Strawberry Perl. The first native build compiles SQLCipher, OpenSSL, and Wasmtime and takes several minutes.

```powershell
npm ci
$env:PERL = 'C:\Strawberry\perl\bin\perl.exe'
npm run desktop -- dev
```

The four signed plugin packages are included in `registry/dist`. Provider sign-in needs your own app registrations; see [setup](docs/setup.md). No credentials or OAuth registrations are bundled. `npm run dev` alone is the UI development server; vault operations require the native app.

## Use

1. Add a plugin and finish browser authorization.
2. Select an account and provider resource in **My keys**.
3. Create, retrieve, or import a credential. Values are masked until requested.
4. Register consumers: a local process, a GitHub repository, or a Vercel project.
5. Rotate a supported key. Verify each consumer after deployment, then retire the old key.

Revealed values hide after 30 seconds or loss of focus. Copied values clear after 30 seconds only if the clipboard still contains that value. Export a passphrase-encrypted `.age` backup from Settings before moving devices. Restore uses an empty vault and never overwrites an existing one.

```powershell
cargo run -p getkeys -- list
cargo run -p getkeys -- connect official.aws --config start_url=https://example.awsapps.com/start --config region=eu-west-2
cargo run -p getkeys -- run --project my-app --environment development -- node server.js
```

`getkeys run` passes local bindings directly to the child process without a shell or plaintext `.env` file. `getkeys get <id> --show` deliberately writes a value to stdout. The typed `getkeys rpc` interface accepts a JSON command on stdin; see the generated [command contract](apps/desktop/src/types.ts).

## Validate

```powershell
npx playwright install chromium
npm run check
npm run desktop -- build --debug
npm run test:packaged
npm run test:live
```

`check` runs formatting, signature verification, generated types, production UI build, Clippy, the full Rust test suite, UI tests, native Windows tests, browser accessibility checks, and npm audit. Live checks **fail** when sandbox configuration is missing; they never silently skip. See [live-test configuration](docs/live-tests.md).

CI gates releases. The release workflow depends on CI and successful live provider checks before publishing an installer. Signing the Windows installer with an OS code-signing certificate is not configured yet; marketplace signatures are separate from installer signing.

## Develop plugins

Each provider has one manifest and a pure Wasm adapter under `plugins/`. The marketplace accepts only catalogs signed by the bundled trust root. Installed packages retain their signed catalog; artifacts are verified on load. Adding a provider does not require a new credential-management service or renderer adapter.

For a development catalog, create your own signing key once, then rebuild:

```powershell
cargo run -p xtask -- init-signing
npm run plugins:build
npm run types
```

This intentionally replaces `registry/public-key.hex`. Keep `.local/registry-signing-key.hex` private. Releases of this repository must use the matching maintainer key via `GETKEYS_REGISTRY_SIGNING_KEY`; do not replace the production trust root just to publish an update.

See [architecture](docs/architecture.md) for the plugin boundary and rotation recovery behavior.
