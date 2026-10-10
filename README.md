<div align="center">

<img src="apps/desktop/src-tauri/icons/128x128.png" width="96" alt="GetKeys logo" />

# GetKeys

**All your API keys, in one place. On your device.**

GetKeys lets you create, rotate, copy, and revoke keys without visiting a single dashboard.

[![CI](https://github.com/weilueluo/getkeys/actions/workflows/ci.yml/badge.svg)](https://github.com/weilueluo/getkeys/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/weilueluo/getkeys)](https://github.com/weilueluo/getkeys/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)

[**Download**](https://github.com/weilueluo/getkeys/releases/latest)

</div>

---

## Why GetKeys

- **Zero setup.** Reuses the GitHub, AWS, Vercel, and Supabase CLI sign-ins you already have, or a token you paste.
- **Every key, one list.** Search, group, reveal, copy, or export to `.env` across all your providers.
- **Create and rotate in place.** New keys and safe rotations through each provider's API, no browser tab-hopping.
- **Local and encrypted.** Keys live in an encrypted vault on your device, unlocked by your OS credential store. No cloud, no account.
- **Always within reach.** Lives in the tray. Press <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>K</kbd> anywhere to find and copy a key.
- **Ready for AI agents.** One click connects Claude, Cursor, or VS Code over MCP, with every secret read logged.

## Supported providers

GitHub · AWS · Vercel · Supabase · OpenAI · Anthropic · Gemini · OpenRouter · Cloudflare · GitLab · Groq · Together AI · Mistral AI · DeepSeek · xAI · Cohere · Fireworks AI · Perplexity · Replicate · Hugging Face · Stripe · Resend

Plus **any other service**: paste `NAME=value` pairs and GetKeys keeps them alongside the rest.

## Install

Grab the latest build from [**Releases**](https://github.com/weilueluo/getkeys/releases/latest).

| Platform | App                                       | CLI only                           |
| -------- | ----------------------------------------- | ---------------------------------- |
| Windows  | `GetKeys_x64-setup.exe`                   | `getkeys-windows-x64.exe`          |
| macOS    | `GetKeys_aarch64.dmg` / `GetKeys_x64.dmg` | `getkeys-macos-{arm64,x64}.tar.gz` |
| Linux    | `.AppImage` / `.deb`                      | `getkeys-linux-x64.tar.gz`         |

The desktop installer also adds the `getkeys` command to your PATH.

> [!NOTE]
> Builds are not yet code-signed. On macOS, right-click the app and choose **Open** the first time. On Windows, choose **More info → Run anyway** if SmartScreen appears.

### Extensions (in progress)

- **Browser** (Chrome, Edge): spots a freshly created key on a provider's page and offers to save it.
- **VS Code**: browse keys, insert references, write `.env` files, and run tasks with keys injected.

## From the terminal

The `getkeys` CLI shares the same vault as the app.

```sh
getkeys ls
getkeys run --group backend -- node server.js
getkeys env --group web -o .env
getkeys new supabase --name "App server"
```

Run `getkeys --help` for everything else.

## Privacy

GetKeys has no servers and collects nothing. Keys are stored in a SQLCipher vault whose key is held by Windows Credential Manager, macOS Keychain, or the Linux Secret Service. Revealed values hide after 30 seconds and copied values are cleared from the clipboard. Provider integrations are signed WebAssembly packages that run sandboxed.

## Build from source

Requires Node 24, Rust (see `rust-toolchain.toml`) with the `wasm32-unknown-unknown` target, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS. On Windows, also install Strawberry Perl and set `PERL`.

```sh
npm ci
npm run desktop -- dev     # run the app
npm run check              # lint and test everything
```

## License

[MIT](LICENSE) © Weilue Luo
