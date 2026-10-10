import { useState } from 'react';
import { Copy, Download, FolderOpen, Plus, Upload, X } from 'lucide-react';
import type { Controller } from './controller';
import { request } from './bridge';
import { Field, Modal, PageHeader } from './ui';
import { ShellSettings } from './ShellSettings';
import { AgentConnect } from './AgentConnect';
export function SettingsPanel({ snapshot, busy, perform }: Controller) {
  const [backup, setBackup] = useState<'export' | 'restore'>();
  const [addingFolder, setAddingFolder] = useState(false);
  const folders = snapshot.project_folders;
  const saveFolders = (next: string[], message: string) =>
    perform(() => request({ kind: 'set_project_folders', folders: next }), message);
  return (
    <>
      <PageHeader title="Settings" />
      <section className="settings-section">
        <div className="settings-label">
          <h2>Local vault</h2>
          <p>
            Encrypted with SQLCipher. The vault key is held by your operating system’s credential
            store, and connection tokens never reach this window.
          </p>
        </div>
        <div className="settings-content">
          <div className="settings-row">
            <div className="settings-text">
              <strong>Vault location</strong>
              <code className="path" title={snapshot.vault_path}>
                {snapshot.vault_path}
              </code>
            </div>
            <div className="button-row">
              <button
                className="icon-button outline"
                title="Copy path"
                aria-label="Copy vault path"
                onClick={() =>
                  void perform(
                    () => navigator.clipboard.writeText(snapshot.vault_path),
                    'Path copied',
                  )
                }
              >
                <Copy size={14} strokeWidth={1.75} />
              </button>
              <button
                className="button outline"
                disabled={busy}
                onClick={() => void perform(() => request({ kind: 'open_vault_folder' }))}
              >
                <FolderOpen size={14} strokeWidth={1.75} /> Open folder
              </button>
            </div>
          </div>
          <div className="settings-row">
            <div>
              <strong>Encrypted backup</strong>
              <small>A passphrase-protected .age file you can restore on another device.</small>
            </div>
            <div className="button-row">
              <button className="button outline" onClick={() => setBackup('restore')}>
                <Upload size={14} strokeWidth={1.75} /> Restore
              </button>
              <button className="button outline" onClick={() => setBackup('export')}>
                <Download size={14} strokeWidth={1.75} /> Export
              </button>
            </div>
          </div>
        </div>
      </section>
      <section className="settings-section">
        <div className="settings-label">
          <h2>Project folders</h2>
          <p>
            Refresh also reads .env files in these folders and their projects, and lists the
            provider keys it finds.
          </p>
        </div>
        <div className="settings-content">
          {folders.length === 0 && (
            <div className="settings-row">
              <small>No folders yet.</small>
            </div>
          )}
          {folders.map((folder) => (
            <div className="settings-row" key={folder}>
              <code className="path" title={folder}>
                {folder}
              </code>
              <button
                className="icon-button outline"
                aria-label={`Remove ${folder}`}
                title="Remove"
                disabled={busy}
                onClick={() =>
                  void saveFolders(
                    folders.filter((f) => f !== folder),
                    'Folder removed',
                  )
                }
              >
                <X size={14} strokeWidth={1.75} />
              </button>
            </div>
          ))}
          <div className="settings-row">
            <button className="button outline" onClick={() => setAddingFolder(true)}>
              <Plus size={14} strokeWidth={1.75} /> Add folder
            </button>
          </div>
        </div>
      </section>
      <ShellSettings busy={busy} perform={perform} />
      <AgentConnect busy={busy} perform={perform} />
      {addingFolder && (
        <Modal title="Add a project folder" onClose={() => setAddingFolder(false)}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const path = String(new FormData(e.currentTarget).get('path')).trim();
              void saveFolders([...folders, path], 'Folder added').then((result) => {
                if (result !== undefined) setAddingFolder(false);
              });
            }}
          >
            <Field label="Folder path">
              <input name="path" placeholder="C:\Users\you\code" required autoFocus />
            </Field>
            <button className="button full" disabled={busy}>
              Add folder
            </button>
          </form>
        </Modal>
      )}
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
    </>
  );
}
