# GetKeys for VS Code

Use the keys you keep in [GetKeys](https://github.com/weilueluo/getkeys) without pasting them into files.

- **Keys view** — keys grouped by provider, with a warning on keys that need attention.
- **Insert Key Reference** — inserts a `getkeys://provider/key[/field]` reference at the cursor. References are safe to commit; `getkeys run` resolves them.
- **Copy Key** — copies a value to the clipboard (cleared after 45 s by default).
- **Write .env** — writes the keys of one or more groups to a `.env` file.
- **Run Task with Keys** — runs a command as a task with keys injected as environment variables (`getkeys run`).
- **Hover** — hovering a `getkeys://` reference shows the key's name, provider and groups. Values are never shown.

Tasks can also be declared in `tasks.json`:

```json
{
  "type": "getkeys",
  "command": "npm test",
  "groups": ["Work"],
  "label": "test with keys"
}
```

## Requirements

The extension talks only to the `getkeys` CLI, which the GetKeys desktop installer adds to `PATH`. Set `getkeys.cliPath` if it lives elsewhere.

## Settings

| Setting                         | Default   | Description                                     |
| ------------------------------- | --------- | ----------------------------------------------- |
| `getkeys.cliPath`               | `getkeys` | Path to the CLI.                                |
| `getkeys.clipboardClearSeconds` | `45`      | Clear a copied value after N seconds (0 = off). |
