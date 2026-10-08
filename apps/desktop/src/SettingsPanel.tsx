import { useState } from 'react';
import { Download, ShieldCheck, Upload } from 'lucide-react';
import type { Controller } from './controller';
import { request } from './bridge';
import { Field, Modal, ProviderMark } from './ui';
export function SettingsPanel({
  snapshot,
  busy,
  perform,
  reconnect,
}: Controller & { reconnect: (id: string) => void }) {
  const [backup, setBackup] = useState<'export' | 'restore'>();
  const [disconnect, setDisconnect] = useState<string>();
  return (
    <>
      <div className="page-heading">
        <div className="eyebrow">ON YOUR TERMS</div>
        <h1>Settings</h1>
        <p>Your vault, connections, and recovery options.</p>
      </div>
      <section className="settings-section">
        <h2>
          <ShieldCheck size={18} /> Local vault
        </h2>
        <p>
          Credentials are encrypted with SQLCipher. The vault key lives in your operating system’s
          credential store. Connection tokens stay out of the interface.
        </p>
        <div className="button-row">
          <button className="button secondary" onClick={() => setBackup('export')}>
            <Download size={15} /> Export encrypted backup
          </button>
          <button className="button secondary" onClick={() => setBackup('restore')}>
            <Upload size={15} /> Restore backup
          </button>
        </div>
      </section>
      <section className="settings-section">
        <h2>Provider setup</h2>
        {snapshot.setup.length ? (
          <>
            {snapshot.setup.map((s) => (
              <p className="setup-note" key={s}>
                {s}
              </p>
            ))}
            <p className="quiet small-text">
              OAuth app registrations belong to the app operator. Set the configuration before
              launching GetKeys, then restart the app. AWS uses your IAM Identity Center start URL
              and region.
            </p>
          </>
        ) : (
          <p>
            Browser authorization is configured. Your provider still controls the permissions you
            can grant.
          </p>
        )}
      </section>
      <section className="settings-section">
        <h2>Accounts</h2>
        {snapshot.connections.map((c) => (
          <div className="settings-account" key={c.id}>
            <ProviderMark
              small
              name={
                snapshot.plugins.find((p) => p.manifest.id === c.plugin_id)?.manifest.name ?? ''
              }
            />
            <div>
              <strong>{c.name}</strong>
              <small>
                {c.state === 'needs_reauth'
                  ? 'Reconnect required'
                  : c.state === 'disconnected'
                    ? 'Disconnected · saved keys available'
                    : 'Connected'}
              </small>
            </div>
            <button className="text-button" disabled={busy} onClick={() => reconnect(c.id)}>
              Reconnect
            </button>
            {c.state !== 'disconnected' && (
              <button className="text-button" disabled={busy} onClick={() => setDisconnect(c.id)}>
                Disconnect
              </button>
            )}
          </div>
        ))}
        {snapshot.connections.length === 0 && <p className="quiet">No accounts connected.</p>}
      </section>
      <section className="settings-section">
        <h2>Installed plugins</h2>
        {snapshot.plugins
          .filter((p) => p.installed)
          .map((p) => (
            <div className="settings-account" key={p.manifest.id}>
              <ProviderMark small name={p.manifest.name} />
              <div>
                <strong>{p.manifest.name}</strong>
                <small>Version {p.manifest.version}</small>
              </div>
              <button
                className="text-button"
                disabled={busy}
                onClick={() =>
                  void perform(
                    () => request({ kind: 'uninstall', plugin_id: p.manifest.id }),
                    'Plugin removed',
                  )
                }
              >
                Remove
              </button>
            </div>
          ))}
      </section>
      {backup && (
        <Modal
          title={backup === 'export' ? 'Export encrypted backup' : 'Restore your vault'}
          onClose={() => setBackup(undefined)}
        >
          <p>
            {backup === 'export'
              ? 'Choose a new .age file and a passphrase of at least 12 characters. Keep the passphrase separately from the backup.'
              : 'Restore requires an empty vault. It never merges or overwrites credentials.'}
          </p>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const d = new FormData(e.currentTarget);
              void perform(
                () =>
                  request({
                    kind: backup,
                    path: String(d.get('path')),
                    passphrase: String(d.get('passphrase')),
                  }),
                backup === 'export' ? 'Encrypted backup saved' : 'Backup restored',
              ).then((result) => {
                if (result !== undefined) setBackup(undefined);
              });
            }}
          >
            <Field label="Full file path">
              <input name="path" placeholder="C:\Backups\getkeys.age" required />
            </Field>
            <Field label="Backup passphrase">
              <input
                type="password"
                name="passphrase"
                minLength={backup === 'export' ? 12 : 1}
                autoComplete="new-password"
                required
              />
            </Field>
            <button className="button full" disabled={busy}>
              {backup === 'export' ? 'Export backup' : 'Restore vault'}
            </button>
          </form>
        </Modal>
      )}
      {disconnect && (
        <Modal title="Disconnect this account?" onClose={() => setDisconnect(undefined)}>
          <p>
            This removes the stored connection token. Saved credential values stay in your local
            vault. You can also revoke the app’s access in your provider settings.
          </p>
          <button
            className="button full"
            disabled={busy}
            onClick={() =>
              void perform(
                () => request({ kind: 'disconnect', connection_id: disconnect }),
                'Account disconnected',
              ).then((result) => {
                if (result !== undefined) setDisconnect(undefined);
              })
            }
          >
            Disconnect account
          </button>
        </Modal>
      )}
    </>
  );
}
