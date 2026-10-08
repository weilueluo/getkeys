import { ArrowUpRight, Check, Plus, ShieldCheck } from 'lucide-react';
import { useState } from 'react';
import type { PluginView } from './types';
import type { Controller } from './controller';
import { ProviderMark, humanize } from './ui';
import { request } from './bridge';

export function Marketplace({
  snapshot,
  busy,
  perform,
  search,
  connect,
}: Controller & { search: string; connect: (plugin: PluginView) => void }) {
  const [filter, setFilter] = useState('all');
  const plugins = snapshot.plugins.filter(
    (p) =>
      (filter === 'all' || p.installed) &&
      `${p.manifest.name} ${p.manifest.description}`.toLowerCase().includes(search.toLowerCase()),
  );
  return (
    <>
      <div className="page-heading">
        <div>
          <div className="eyebrow">YOUR TOOLS, TOGETHER</div>
          <h1>One place for every key.</h1>
          <p>Add a provider, connect your account, and keep your credentials close.</p>
        </div>
      </div>
      <div className="section-toolbar">
        <div className="tabs" aria-label="Plugin filter">
          {['all', 'installed'].map((f) => (
            <button key={f} className={filter === f ? 'active' : ''} onClick={() => setFilter(f)}>
              {f === 'all' ? 'All plugins' : 'Installed'}{' '}
              <span>
                {f === 'all'
                  ? snapshot.plugins.length
                  : snapshot.plugins.filter((p) => p.installed).length}
              </span>
            </button>
          ))}
        </div>
        <span className="quiet small-text">
          <ShieldCheck size={14} /> Verified packages
        </span>
      </div>
      <div className="plugin-grid">
        {plugins.map((p) => (
          <article className="plugin-card" key={p.manifest.id}>
            <div className="card-top">
              <ProviderMark name={p.manifest.name} />
              <span className="badge">
                {p.installed ? (
                  <>
                    <Check size={12} /> Installed
                  </>
                ) : (
                  'Official'
                )}
              </span>
            </div>
            <h2>{p.manifest.name}</h2>
            <p>{p.manifest.description}</p>
            <div className="card-capabilities">
              <span>
                {p.manifest.operations.includes('create_credential')
                  ? 'Automatic rotation'
                  : 'Guided replacement'}
              </span>
              {p.manifest.operations.includes('write_secret') && <span>Secret delivery</span>}
            </div>
            <details className="permissions">
              <summary>Permissions & details</summary>
              <p>
                Published by {p.manifest.publisher} · v{p.manifest.version}
              </p>
              <p>Network: {p.manifest.hosts.join(', ')}</p>
              <p>{p.manifest.operations.map(humanize).join(', ')}</p>
            </details>
            <div className="card-footer">
              <span className="small-text quiet">
                {p.manifest.auth.kind === 'aws_sso'
                  ? 'IAM Identity Center'
                  : 'Browser authorization'}
              </span>
              <button
                disabled={busy}
                className={p.installed ? 'button secondary compact' : 'button compact'}
                onClick={() => connect(p)}
              >
                {p.installed ? (
                  <>
                    Connect <ArrowUpRight size={14} />
                  </>
                ) : (
                  <>
                    Add plugin <Plus size={14} />
                  </>
                )}
              </button>
            </div>
            {p.update_available && (
              <button
                disabled={busy}
                className="text-button"
                onClick={() =>
                  void perform(
                    () =>
                      request({
                        kind: 'install',
                        plugin_id: p.manifest.id,
                        accept_permissions: true,
                      }),
                    'Plugin updated',
                  )
                }
              >
                Install update with the permissions above
              </button>
            )}
          </article>
        ))}
      </div>
      {plugins.length === 0 && <p className="no-results">No plugins match your search.</p>}
      <div className="marketplace-note">
        <span className="note-rule" />
        <p>
          Your keys stay in your encrypted local vault.
          <br />
          <span className="quiet">
            Plugins only access the provider hosts and operations you approve.
          </span>
        </p>
      </div>
    </>
  );
}
