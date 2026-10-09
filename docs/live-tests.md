# Live validation

OAuth app registrations and sandbox accounts were not configured when this project was created. Automated local tests use fixtures; they do not prove a real account's permissions or OAuth approval. `npm run test:live` exits with a blocking error when any required configuration is absent, before making requests.

## Automated provider API checks

Use dedicated, disposable resources. Set the following environment variables without committing them:

| Provider | Required variables |
| --- | --- |
| GitHub | `GETKEYS_LIVE_GITHUB_TOKEN`, `GETKEYS_LIVE_GITHUB_REPO` (`owner/repo`) |
| AWS | `GETKEYS_LIVE_AWS_ACCESS_KEY_ID`, `GETKEYS_LIVE_AWS_SECRET_ACCESS_KEY`, `GETKEYS_LIVE_AWS_USER` |
| Supabase | `GETKEYS_LIVE_SUPABASE_TOKEN`, `GETKEYS_LIVE_SUPABASE_PROJECT` (project ref) |
| Vercel | `GETKEYS_LIVE_VERCEL_TOKEN`, `GETKEYS_LIVE_VERCEL_PROJECT` (project ID) |

Optional: `GETKEYS_LIVE_AWS_SESSION_TOKEN`, `GETKEYS_LIVE_VERCEL_TEAM_ID`. Supabase needs a Management API access token, not a project's API key. Prefer access tokens obtained through the configured OAuth registrations so their permissions match the shipped connection flow.

The AWS target must be an IAM user named `getkeys-test-*` with **zero existing access keys**. The test authenticates using separate management credentials scoped to that user. GitHub needs repository Actions-secret write access. Vercel needs project environment-variable write access. Supabase needs API-key read/create/delete permissions on the sandbox project.

```powershell
$env:GETKEYS_LIVE_ALLOW_WRITES = 'true'
npm run test:live
```

This explicitly permits the runner to:

- Validate the four account identities.
- Create and validate two AWS keys, confirm the two-key limit, deactivate one, and delete both.
- Create, validate, list, reveal, and delete modern Supabase secret keys.
- Write unique test-only GitHub Actions secrets and Vercel preview variables, check metadata, and delete them.

Cleanup runs after failed assertions where the provider returned the created IDs. If a creation response is lost, inspect the dedicated resource before retrying. Remove orphaned `getkeys-live-*` keys or `GETKEYS_LIVE_*` variables. The runner never removes pre-existing keys to make room. Do not run two live suites against the same sandbox at once.

## Browser and consumer acceptance

With [provider setup](setup.md) complete, check these behaviors with real accounts:

1. Add each provider and authorize in the system browser. For AWS, choose an assigned account/role. Test denial and expiration too.
2. Reconnect the same account from its provider in **Providers**. Existing credentials and bindings must remain attached. A different identity or Vercel team must be rejected.
3. Restart the app and confirm the connection persists without requesting authorization again. Test Supabase refresh after access-token expiry.
4. Rotate a disposable AWS or Supabase key used by a disposable application. Redeploy/restart the consumer, make an authenticated request, then click Verify. Confirm the old key remains usable until retirement.
5. For GitHub/Vercel personal tokens, create a new token in the provider, import it as a new key, and choose it under **Replace in provider**. Distribute it, verify consumers, revoke the old token in the provider, then confirm manual retirement in GetKeys. Integration consent is not permission to create arbitrary personal tokens.

These steps are distinct from API contract tests. A secret-writing API cannot attest that a running consumer deployed successfully.

## CI release configuration

Create the repository environment `live-sandbox`. Put token/access-key values in its secrets; put repository, IAM user, project, and optional team IDs in environment variables with the names above. A `v*` tag triggers checks, then the live suite, then packaging/publication. Missing values fail the live job and prevent release. Windows installer code signing needs a separately configured certificate; it is not implied by the marketplace's Ed25519 signature.
