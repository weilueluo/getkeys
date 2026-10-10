# Extensions

Two optional extensions sit on top of the desktop app and the `getkeys` CLI. Both live in `extensions/` (a self-contained npm workspace, separate from the root one).

```powershell
npm run extensions:build   # installs extensions/ deps on first run, builds both
npm run extensions:test    # typecheck + unit tests (vitest)
```

Outputs:

| Artifact                                 | What                                    |
| ---------------------------------------- | --------------------------------------- |
| `extensions/browser/dist/`               | Unpacked Chrome/Edge extension (MV3)    |
| `extensions/browser/getkeys-browser.zip` | Same, zipped for release / store upload |
| `extensions/vscode/getkeys.vsix`         | VS Code extension package               |

## Browser capture (Chrome / Edge)

Most providers show a new API key once. The extension notices a provider key on the page — in a dialog, a code block, a read-only field, or when you copy it — and offers to save it into GetKeys before you close the dialog.

### Install

1. Install the GetKeys desktop app (it puts `getkeys` on `PATH`).
2. Register the native messaging host once:

   ```powershell
   getkeys native-host install
   ```

   This registers `dev.getkeys.host` for Chrome and Edge, allowing only the GetKeys extension (ID `mhdeafblmlpikemoliokafdegcdhoeed`, fixed by the manifest `key`).

3. Open `chrome://extensions` (or `edge://extensions`), enable **Developer mode**, choose **Load unpacked** and pick `extensions/browser/dist` (or the extracted release zip).

The toolbar popup shows **Connected** when the host answers. If it says _Not connected_, run step 2 again and press **Retry**.

### Use

- **Card** — when a fresh key appears, a small card in the bottom-right says _"OpenAI key found · Save to GetKeys"_ with a suggested name. **Save** stores it; **Dismiss** stops prompting for that key on this page. Turn the card off in the popup (_Offer to save keys shown on pages_).
- **Popup** — lists every key detected on the current tab (each with a name and **Save**), and **Save selection** saves whatever text you have selected (provider detected when it matches a known pattern, otherwise saved as a custom key).
- The toolbar badge shows how many keys were detected on the tab.

Detection uses the `key_pattern` of each installed provider, fetched from the host (`{"type":"patterns"}`) and cached in extension storage. Masked keys (`sk-…abcd`) and documentation placeholders are ignored.

### Privacy

- Key values go to exactly one place: the local native host (`getkeys native-host`), and only when you press **Save**. There is no network access, analytics or remote code.
- Pages are scanned locally inside the content script. Values stay in that tab's memory, deduplicated by a hash; they are never written to extension storage or logged.
- The page URL sent with a save is stripped of its query string and fragment.
- The card renders in a closed shadow root so page scripts cannot read or restyle it.
- Permissions: `nativeMessaging` (talk to GetKeys), `storage` (cached patterns and the card setting), `activeTab` (popup ↔ current tab); the content script runs on all sites because keys can appear on any provider's dashboard.

## VS Code

The extension uses only the `getkeys` CLI (`getkeys.cliPath`, default `getkeys` on `PATH`). Install it with:

```powershell
code --install-extension extensions/vscode/getkeys.vsix
```

- **GetKeys view** (key icon in the activity bar) — keys grouped by provider; keys with a problem show a warning and the view shows a badge. Title buttons: run a task, reload; overflow: _Refresh from Providers_ (`getkeys refresh`), _Write .env_. Inline buttons on a key insert its reference or copy its value.
- **GetKeys: Insert Key Reference** — inserts `getkeys://<provider>/<key>[/<field>]` at every cursor.
- **GetKeys: Copy Key** — `getkeys get <ref> --field F --show` → clipboard; cleared after `getkeys.clipboardClearSeconds` (45 s) if unchanged.
- **GetKeys: Write .env** — choose groups and/or keys, then a file → `getkeys env … -o FILE`.
- **GetKeys: Run Task with Keys** — runs a shell command as a VS Code task via `getkeys run [--group G]… [--key REF]… -- <shell> <command>`. Tasks can also be declared in `tasks.json` with `"type": "getkeys"`, `"command"`, `"groups"`, `"keys"`. Disabled in untrusted workspaces.
- **Hover** — hovering a `getkeys://` reference shows name, provider, field, groups and problems. Values are never shown.
- A status bar warning appears when the CLI cannot be found.

The extension never stores values; the only command that reads one is _Copy Key_.

## Development

- Pure logic lives in `extensions/browser/src/lib.ts` and `extensions/vscode/src/lib.ts`, covered by `extensions/*/test/*.test.ts`.
- Icons are drawn at build time by `extensions/scripts/icons.mjs` (same mark as the desktop app); the release zip is written by `extensions/scripts/zip.mjs`.
- Native host contract (`dev.getkeys.host`, Chrome length-prefixed JSON):
  - `{"type":"ping"}` → `{"ok":true,"version":"…"}`
  - `{"type":"patterns"}` → `{"ok":true,"patterns":[{"provider","name","pattern"}]}` (JS regex source)
  - `{"type":"save","provider":id|null,"name","value","url"}` → `{"ok":true,"id"}` or `{"ok":false,"error"}`
