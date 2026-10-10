# Live validation

OAuth app registrations and sandbox accounts were not configured when this project was created. Automated local tests use fixtures; they do not prove a real account's permissions or OAuth approval. `npm run test:live` exits with a blocking error when any required configuration is absent, before making requests.

## Automated provider API checks

Use dedicated, disposable resources. Set the following environment variables without committing them:

| Provider | Required variables                                                                              |
| -------- | ----------------------------------------------------------------------------------------------- |
| GitHub   | `GETKEYS_LIVE_GITHUB_TOKEN`                                                                     |
| AWS      | `GETKEYS_LIVE_AWS_ACCESS_KEY_ID`, `GETKEYS_LIVE_AWS_SECRET_ACCESS_KEY`, `GETKEYS_LIVE_AWS_USER` |
| Supabase | `GETKEYS_LIVE_SUPABASE_TOKEN`, `GETKEYS_LIVE_SUPABASE_PROJECT` (project ref)                    |
| Vercel   | `GETKEYS_LIVE_VERCEL_TOKEN`                                                                     |

Optional: `GETKEYS_LIVE_AWS_SESSION_TOKEN`, `GETKEYS_LIVE_VERCEL_TEAM_ID`. Supabase needs a Management API access token, not a project's API key. Prefer access tokens obtained through the configured OAuth registrations so their permissions match the shipped connection flow.

The AWS target must be an IAM user named `getkeys-test-*` with **zero existing access keys**. The test authenticates using separate management credentials scoped to that user. GitHub needs only to read the signed-in user. Vercel needs permission to create and delete account tokens. Supabase needs API-key read/create/delete permissions on the sandbox project.

```powershell
$env:GETKEYS_LIVE_ALLOW_WRITES = 'true'
npm run test:live
```

This explicitly permits the runner to:

- Validate the four account identities.
- Create and validate two AWS keys, confirm the two-key limit, deactivate one, and delete both.
- Create, validate, list, reveal, and delete modern Supabase secret keys.
- Create, validate, list, and revoke a Vercel account token.

Cleanup runs after failed assertions where the provider returned the created IDs. If a creation response is lost, inspect the dedicated resource before retrying. Remove orphaned `getkeys-live-*` keys. The runner never removes pre-existing keys to make room. Do not run two live suites against the same sandbox at once.

## Browser acceptance

With [provider setup](setup.md) complete, check these behaviors with real accounts:

1. Add each provider and authorize in the system browser. For AWS, choose an assigned account/role. Test denial and expiration too.
2. Reconnect the same account from its provider in **Providers**. Existing keys must remain attached. A different identity or Vercel team must be rejected.
3. Restart the app and confirm the connection persists without requesting authorization again. Test Supabase refresh after access-token expiry.
4. Rotate a disposable AWS or Supabase key with **Rotate** in its details. Confirm the new key works, keeps the old one's name and groups, and the old key stops working.
5. Make a GitHub personal token with **New key**: GitHub's new-token page must open filled in, and copying the token there must save it in GetKeys without pasting. Rotate it the same way and confirm GetKeys revokes the old token through the API. Do the same for a Vercel token. Integration consent is not permission to create arbitrary personal tokens.

## CI release configuration

Create the repository environment `live-sandbox`. Put token/access-key values in its secrets; put the IAM user, project, and optional team IDs in environment variables with the names above. Then set the repository variable `GETKEYS_LIVE_ENABLED` to `true`. A `v*` tag triggers checks, then the live suite, then packaging/publication. With the variable set, missing values fail the live job and prevent release; without it, the live job is skipped and releases rely on CI alone. Windows installer code signing needs a separately configured certificate; it is not implied by the marketplace's Ed25519 signature.
