# Provider setup

No setup is required. **Add** on a provider signs in with every session for it that already exists on this machine:

| Provider | Found from                                                                                                                            |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| GitHub   | Each `gh` CLI account (`gh auth token`), `GH_TOKEN`, `GITHUB_TOKEN`                                                                   |
| Vercel   | The Vercel CLI session (refreshed with `vercel whoami` when it expires), `VERCEL_TOKEN`                                               |
| Supabase | The Supabase CLI token (OS keychain or `~/.supabase/access-token`), `SUPABASE_ACCESS_TOKEN`                                           |
| AWS      | Static keys in `~/.aws/credentials`, `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`, and IAM Identity Center profiles in `~/.aws/config` |

Sessions are read only when you add a provider or refresh keys, and each account is checked with the provider before it is saved. Profiles that resolve to the same identity become one account. A connection made from a CLI session re-reads that session when its token expires, so signing in again with the CLI also renews GetKeys. **Refresh** on the Keys page picks up new sessions for providers you've added, but never brings back an account you disconnected. Both show progress in a window you can cancel.

If no session works, **Add** opens a dialog with a link to the provider's token page; create a token there and paste it. Once an account is added, its management key is listed under the provider in **Providers**; a key found on this device is listed under **Keys** as well. For Vercel, **Create dedicated key** swaps the CLI session for a token GetKeys creates and later rotates.

## Browser sign-in (optional)

Browser sign-in appears only when its registration is configured. Set environment variables before launching GetKeys. Restart after changing them. `.env` is ignored by Git and is not loaded automatically. For local development, copy `.env.example` to `.env` and launch with:

```powershell
node --env-file=.env scripts/desktop.mjs dev
```

Keep client secrets on the broker. A desktop binary cannot protect an embedded OAuth client secret.

## GitHub

Create a GitHub OAuth App, enable its device flow, and set `GETKEYS_GITHUB_CLIENT_ID` to its public client ID. No client secret is needed by the device flow. GetKeys requests `repo read:user` for repository inventory and Actions secret updates. Use an app registration appropriate to the accounts you intend to support.

Add GitHub, enter the displayed device code, and authorize in your browser. Personal access tokens are created in GitHub and imported into GetKeys; GitHub does not provide a general PAT-value retrieval API. [Device-flow documentation](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow).

## AWS

Configure IAM Identity Center, assign an account role to your user, and supply its start URL and commercial region in GetKeys. GetKeys registers a public OIDC client, opens device authorization, then lets you select an assigned role. AWS China and GovCloud partitions are outside this provider's scope.

The selected role needs `iam:ListUsers` and, for the IAM users it manages, `iam:ListAccessKeys`, `iam:CreateAccessKey`, `iam:UpdateAccessKey`, and `iam:DeleteAccessKey`. Scope write permissions to the intended users. Candidate validation uses STS `GetCallerIdentity`. Never use root access keys. AWS permits two access keys per IAM user; GetKeys checks the limit before creation. [Identity Center OIDC](https://docs.aws.amazon.com/singlesignon/latest/OIDCAPIReference/Welcome.html), [IAM key management](https://docs.aws.amazon.com/IAM/latest/UserGuide/id_credentials_access-keys.html).

## Supabase and Vercel broker

Run `getkeys-auth-broker` as a single instance behind HTTPS. Its sessions are short lived and held only in memory. A restart cancels pending authorization. Configure your reverse proxy to limit request rates and avoid logging callback query strings, request bodies, or token responses. Do not put a general request-logging middleware in front of the broker.

| Variable                                       | Purpose                                      |
| ---------------------------------------------- | -------------------------------------------- |
| `GETKEYS_BROKER_PUBLIC_URL`                    | HTTPS origin, without a path                 |
| `GETKEYS_BROKER_LISTEN`                        | Listen address; defaults to `127.0.0.1:8787` |
| `SUPABASE_CLIENT_ID`, `SUPABASE_CLIENT_SECRET` | Supabase **Management OAuth** registration   |
| `VERCEL_CLIENT_ID`, `VERCEL_CLIENT_SECRET`     | Vercel integration registration              |
| `VERCEL_INTEGRATION_SLUG`                      | Vercel integration URL slug                  |

Configure at least one registration. Set `GETKEYS_BROKER_URL` in the desktop environment to this broker's origin. Local HTTP is accepted only for `localhost` or `127.0.0.1` development.

```powershell
# After setting broker environment variables in this terminal:
cargo run -p getkeys-auth-broker
```

Register these exact redirect URLs, replacing the origin:

- Supabase: `https://broker.example.com/callback/supabase`
- Vercel: `https://broker.example.com/callback/vercel`

For Supabase, create an OAuth App in organization settings and configure the Management API permissions for organizations, projects, and API keys. This is separate from Supabase Auth sign-in for an application's users. The broker uses PKCE and client authentication during code exchange and refresh. [Supabase integration guide](https://supabase.com/docs/guides/integrations/build-a-supabase-oauth-integration).

For Vercel, create a connectable-account integration. Enable read access to user/project information and write access to project environment variables; global-project-env-vars is needed to manage variables outside the integration's ownership. The browser starts at `/integrations/<slug>/new`. Vercel integration OAuth does not grant the documented personal-token-management endpoints, so this provider guides token creation and replacement through Vercel's dashboard. [Integration creation](https://vercel.com/docs/integrations/create-integration), [REST permissions](https://vercel.com/docs/integrations/create-integration/vercel-api-integrations).

The callback consumes its state once. The desktop's separate random claim secret binds the result to the initiating device. Claims can be retried during the ten-minute window to survive a lost response; tokens are removed when the session expires. Tokens are saved in the encrypted local vault before the app marks the connection complete.

## Distribution

The initial marketplace is bundled and curated. `getkeys marketplace <https-base-url/> <new-directory>` downloads and verifies a signed catalog, then `GETKEYS_REGISTRY` selects that catalog on the next launch. The trust root stays compiled into the host. Provider updates that add hosts, authentication modes, or operations require review; active rotations block provider updates.
