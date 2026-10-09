import { ArrowUpRight } from 'lucide-react';
import type { Discovery, PluginView } from './types';
import { Field, inputs, Modal } from './ui';

const values = (form: HTMLFormElement) =>
  Object.fromEntries([...new FormData(form)].map(([k, v]) => [k, String(v)]));

export function ConnectDialog({
  plugin,
  found,
  mode,
  busy,
  onClose,
  onLocal,
  onToken,
  onTokenPage,
  onBrowser,
}: {
  plugin: PluginView;
  found: Discovery[];
  /** Connect a new account, sign an existing one in again, or swap its sign-in key. */
  mode: 'connect' | 'reconnect' | 'rotate';
  busy: boolean;
  onClose: () => void;
  onLocal: (d: Discovery) => void;
  onToken: (fields: Record<string, string>) => void;
  onTokenPage: () => void;
  onBrowser: (config: Record<string, string>) => void;
}) {
  const name = plugin.manifest.name;
  const fresh = mode === 'connect';
  const local = !fresh ? [] : found.filter((d) => d.plugin_id === plugin.manifest.id);
  const browser = plugin.browser_sign_in && fresh;
  const tokenFields = inputs(plugin.manifest.account_fields);
  const token = tokenFields.length > 0 && !!plugin.manifest.token_page;
  return (
    <Modal
      title={
        mode === 'rotate'
          ? `Rotate ${name} sign-in key`
          : `${fresh ? 'Connect' : 'Reconnect'} ${name}`
      }
      onClose={onClose}
    >
      {local.length > 0 && (
        <section className="connect-local">
          <span className="field-label">Signed in on this device</span>
          <div className="choice-list">
            {local.map((d) => (
              <button
                className="choice"
                key={d.id}
                disabled={busy || d.connected}
                onClick={() => onLocal(d)}
              >
                <strong>{d.account || d.source}</strong>
                <span className="mono">
                  {d.connected ? 'Connected' : d.account ? d.source : 'Use this sign-in'}
                </span>
              </button>
            ))}
          </div>
        </section>
      )}
      {token && (
        <details className="connect-option" open={!local.length}>
          <summary>{local.length ? 'Use a token instead' : 'Paste a token'}</summary>
          <p className="quiet">
            {mode === 'rotate'
              ? `Create a new token on ${name}, paste it here, then revoke the old one.`
              : `Create a token on ${name}, copy it, and paste it below.`}{' '}
            <button type="button" className="text-button signal" onClick={onTokenPage}>
              Open {name} tokens <ArrowUpRight size={13} />
            </button>
          </p>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              onToken(values(e.currentTarget));
            }}
          >
            {tokenFields.map((f) => (
              <Field key={f.key} label={f.label}>
                <input
                  name={f.key}
                  type={f.secret ? 'password' : 'text'}
                  placeholder={f.placeholder}
                  required={f.required}
                  autoComplete="off"
                  spellCheck={false}
                />
              </Field>
            ))}
            <button className="button full" disabled={busy}>
              {mode === 'rotate' ? 'Use new key' : fresh ? 'Connect' : 'Reconnect'}
            </button>
          </form>
        </details>
      )}
      {browser && (
        <details className="connect-option" open={!local.length && !token}>
          <summary>
            {plugin.manifest.auth.kind === 'aws_sso'
              ? 'Sign in with IAM Identity Center'
              : 'Sign in with your browser'}
          </summary>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              onBrowser(values(e.currentTarget));
            }}
          >
            {plugin.manifest.fields.map((f) => (
              <Field key={f.key} label={f.label}>
                <input
                  name={f.key}
                  placeholder={f.placeholder}
                  required={f.required}
                  autoComplete="off"
                  spellCheck={false}
                />
              </Field>
            ))}
            <button className="button secondary full" disabled={busy}>
              Continue in browser <ArrowUpRight size={14} />
            </button>
          </form>
        </details>
      )}
      {!local.length && !token && !browser && (
        <p className="quiet">
          Nothing to connect with yet. Sign in to {name} on this device first.
        </p>
      )}
    </Modal>
  );
}
