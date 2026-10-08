import github from '../plugins/github/manifest.json' with { type: 'json' };
import aws from '../plugins/aws/manifest.json' with { type: 'json' };
import vercel from '../plugins/vercel/manifest.json' with { type: 'json' };
import supabase from '../plugins/supabase/manifest.json' with { type: 'json' };
import type { Manifest, Snapshot } from '../apps/desktop/src/types';
export function emptySnapshot(): Snapshot {
  return {
    plugins: [github, aws, vercel, supabase].map((manifest) => ({
      manifest: manifest as Manifest,
      installed: false,
      update_available: false,
    })),
    connections: [],
    credentials: [],
    bindings: [],
    rotations: [],
    activity: [],
    setup: ['Configure OAuth registrations before connecting accounts.'],
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
    },
  ];
  snapshot.credentials = [
    {
      id: 'credential',
      connection_id: 'connection',
      provider_id: 'key_123',
      name: 'Production API',
      credential_type: 'secret',
      resource: 'abcdefghijklmnopqrst',
      project: 'getkeys',
      environment: 'production',
      created_at: null,
      expires_at: null,
      value_available: true,
      can_rotate: true,
      retired: false,
    },
  ];
  return snapshot;
}
