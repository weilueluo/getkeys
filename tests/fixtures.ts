import github from '../plugins/github/manifest.json' with { type: 'json' };
import aws from '../plugins/aws/manifest.json' with { type: 'json' };
import vercel from '../plugins/vercel/manifest.json' with { type: 'json' };
import supabase from '../plugins/supabase/manifest.json' with { type: 'json' };
import type { ConfigField, Manifest, Snapshot } from '../apps/desktop/src/types';
// Match the host, which fills `ConfigField` serde defaults before sending manifests.
const field = (f: Partial<ConfigField>) =>
  ({ placeholder: '', required: false, secret: true, env: '', ...f }) as ConfigField;
const normalize = (m: object) => {
  const raw = m as Manifest;
  return {
    ...raw,
    fields: raw.fields.map(field),
    credential_fields: raw.credential_fields.map(field),
    account_fields: (raw.account_fields ?? []).map(field),
    token_page: raw.token_page ?? '',
    key_page: raw.key_page ?? '',
    revoke_by_value: raw.revoke_by_value ?? false,
    account_credentials: raw.account_credentials ?? false,
  } as Manifest;
};
export function emptySnapshot(): Snapshot {
  return {
    plugins: [github, aws, vercel, supabase].map((manifest) => ({
      manifest: normalize(manifest),
      installed: false,
      update_available: false,
      browser_sign_in: false,
    })),
    connections: [],
    credentials: [],
    rotations: [],
    activity: [],
    vault_path: 'C:\\Users\\you\\AppData\\Roaming\\GetKeys\\vault.db',
  };
}
export function connectedSnapshot(): Snapshot {
  const snapshot = emptySnapshot();
  snapshot.plugins[3].installed = true;
  snapshot.connections = [
    {
      id: 'connection',
      plugin_id: 'official.supabase',
      name: 'Personal projects',
      state: 'connected',
      config: {},
      source: 'supabase',
    },
  ];
  snapshot.credentials = [
    {
      id: 'credential',
      plugin_id: '',
      connection_id: 'connection',
      provider_id: 'key_123',
      origin: 'listed',
      name: 'Production API',
      credential_type: 'secret',
      resource: 'abcdefghijklmnopqrst',
      resource_name: 'GetKeys',
      project: 'getkeys',
      environment: 'production',
      created_at: null,
      expires_at: null,
      value_available: true,
      can_rotate: true,
      retired: false,
      groups: [],
      problem: null,
      fields: [
        {
          key: 'token',
          label: 'API key',
          env: 'SUPABASE_KEY',
          secret: true,
          value: null,
          available: true,
        },
        {
          key: 'url',
          label: 'Project URL',
          env: 'SUPABASE_URL',
          secret: false,
          value: 'https://abcdefghijklmnopqrst.supabase.co',
          available: true,
        },
      ],
    },
    {
      id: 'signin-connection',
      plugin_id: '',
      connection_id: 'connection',
      provider_id: 'sign-in',
      origin: 'found',
      name: 'Supabase CLI',
      credential_type: 'sign_in',
      resource: '',
      resource_name: '',
      project: '',
      environment: '',
      created_at: null,
      expires_at: null,
      value_available: true,
      can_rotate: false,
      retired: false,
      groups: [],
      problem: null,
      fields: [
        {
          key: 'token',
          label: 'Access token',
          env: 'SUPABASE_ACCESS_TOKEN',
          secret: true,
          value: null,
          available: true,
        },
      ],
    },
  ];
  return snapshot;
}
/** Adds a Vercel account whose token listing has no values, alongside its CLI sign-in. */
export function withVercel(snapshot: Snapshot): Snapshot {
  snapshot.plugins[2].installed = true;
  snapshot.connections.push({
    id: 'vercel',
    plugin_id: 'official.vercel',
    name: 'luo',
    state: 'connected',
    config: {},
    source: 'vercel',
  });
  const base = snapshot.credentials[0];
  const token = (id: string, name: string, type: string, available: boolean) => ({
    ...base,
    id,
    connection_id: 'vercel',
    provider_id: id,
    name,
    credential_type: type,
    resource: '',
    resource_name: '',
    project: '',
    environment: '',
    value_available: available,
    can_rotate: false,
    fields: [
      { key: 'token', label: 'Token', env: 'VERCEL_TOKEN', secret: true, value: null, available },
    ],
  });
  snapshot.credentials.push(
    token('signin-vercel', 'Vercel CLI', 'sign_in', true),
    token('deploy', 'deploy-bot', 'secret', false),
  );
  return snapshot;
}
