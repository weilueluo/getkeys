import { useEffect, useState } from 'react';
import {
  ArrowUpRight,
  Check,
  Copy,
  Eye,
  EyeOff,
  KeyRound,
  Plus,
  RefreshCw,
  RotateCw,
  X,
} from 'lucide-react';
import type {
  Binding,
  ConnectionView,
  CredentialView,
  Manifest,
  Resource,
  Rotation,
} from './types';
import type { Controller } from './controller';
import { copyCredential, request } from './bridge';
import { Empty, Field, Modal, ProviderMark, humanize } from './ui';

type Props = Controller & {
  search: string;
  account: string;
  setAccount: (id: string) => void;
  browse: () => void;
  onRotation: () => void;
};
export function VaultPanel({
  snapshot,
  busy,
  perform,
  search,
  account,
  setAccount,
  browse,
  onRotation,
}: Props) {
  const [resources, setResources] = useState<Resource[]>([]);
  const [resource, setResource] = useState('');
  const [selected, setSelected] = useState('');
  const [modal, setModal] = useState<'import' | 'create' | undefined>();
  const connections = snapshot.connections;
  const connection = connections.find((c) => c.id === account) ?? connections[0];
  const manifest = snapshot.plugins.find((p) => p.manifest.id === connection?.plugin_id)?.manifest;
  const id = connection?.id;
  const connectionState = connection?.state;
  useEffect(() => {
    setResource('');
    setResources([]);
    setSelected('');
    if (!id) return;
    if (connectionState === 'disconnected') {
      const saved = [
        ...new Set(
          snapshot.credentials.filter((c) => c.connection_id === id).map((c) => c.resource),
        ),
      ].map((id) => ({ id, name: id, kind: 'resource' }));
      setResources(saved);
      setResource(saved[0]?.id ?? '');
      return;
    }
    let active = true;
    void perform(() => request<Resource[]>({ kind: 'resources', connection_id: id })).then(
      (data) => {
        if (active && data) {
          setResources(data);
          setResource(data[0]?.id ?? '');
        }
      },
    );
    return () => {
      active = false;
    };
  }, [id, connectionState, perform]);
  useEffect(() => {
    if (id && resource && connectionState !== 'disconnected')
      void perform(() => request({ kind: 'sync', connection_id: id, resource }));
  }, [id, resource, connectionState, perform]);
  const credentials = snapshot.credentials.filter(
    (c) =>
      c.connection_id === id &&
      (!resource || c.resource === resource) &&
      !c.retired &&
      `${c.name} ${c.project} ${c.environment}`.toLowerCase().includes(search.toLowerCase()),
  );
  const credential = snapshot.credentials.find(
    (c) => c.id === selected && c.connection_id === id && c.resource === resource,
  );
  return (
    <>
      <div className="page-heading row">
        <div>
          <div className="eyebrow">YOUR LOCAL VAULT</div>
          <h1>My keys</h1>
          <p>The credentials you need, without the dashboard hopping.</p>
        </div>
        <button className="button secondary" onClick={browse}>
          <Plus size={15} /> Add provider
        </button>
      </div>
      {!connection || !manifest ? (
        <Empty
          title="Make yourself at home."
          action={
            <button className="button" onClick={browse}>
              Browse plugins <ArrowUpRight size={15} />
            </button>
          }
        >
          Connect your first provider to start collecting your keys.
        </Empty>
      ) : (
        <>
          <div className="key-toolbar">
            <div className="resource-select">
              <ProviderMark small name={manifest.name} />
              <select
                aria-label="Account"
                value={connection.id}
                onChange={(e) => setAccount(e.target.value)}
              >
                {connections.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
              </select>
            </div>
            <select
              aria-label="Provider resource"
              value={resource}
              onChange={(e) => {
                setResource(e.target.value);
                setSelected('');
              }}
            >
              <option value="" disabled>
                {resources.length ? 'Choose a resource' : 'No resources available'}
              </option>
              {resources.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name}
                </option>
              ))}
            </select>
            <div className="spacer" />
            <button
              className="icon-button"
              title="Refresh credentials"
              aria-label="Refresh credentials"
              disabled={busy || !resource || connectionState === 'disconnected'}
              onClick={() =>
                void perform(
                  () => request({ kind: 'sync', connection_id: connection.id, resource }),
                  'Credentials refreshed',
                )
              }
            >
              <RefreshCw size={16} />
            </button>
            <button
              className="button secondary compact"
              disabled={busy || !resource || connectionState === 'disconnected'}
              onClick={() => {
                setSelected('');
                setModal('import');
              }}
            >
              Import key
            </button>
            {manifest.operations.includes('create_credential') ? (
              <button
                className="button compact"
                disabled={busy || !resource || connectionState === 'disconnected'}
                onClick={() => setModal('create')}
              >
                <Plus size={14} /> New key
              </button>
            ) : (
              <button
                className="button compact"
                disabled={busy || !resource || connectionState === 'disconnected'}
                onClick={() =>
                  void perform(() =>
                    request<string>({ kind: 'guide', connection_id: connection.id, resource }),
                  ).then((message) => {
                    if (message) window.alert(message);
                  })
                }
              >
                Create in {manifest.name} <ArrowUpRight size={14} />
              </button>
            )}
          </div>
          <div className="key-content">
            <div className="key-list">
              {credentials.length ? (
                <>
                  <div className="list-heading">
                    <span>NAME</span>
                    <span>ENVIRONMENT</span>
                    <span>VALUE</span>
                  </div>
                  {credentials.map((c) => (
                    <button
                      key={c.id}
                      className={`key-row ${selected === c.id ? 'selected' : ''}`}
                      onClick={() => setSelected(c.id)}
                    >
                      <span className="key-name">
                        <KeyRound size={16} />
                        <span>
                          <strong>{c.name}</strong>
                          <small>
                            {c.project ||
                              resources.find((r) => r.id === c.resource)?.name ||
                              c.resource}
                          </small>
                        </span>
                      </span>
                      <span className="environment-tag">{c.environment || 'Unassigned'}</span>
                      <span className={`value-status ${c.value_available ? 'available' : ''}`}>
                        {c.value_available ? 'Stored' : 'Not stored'}
                      </span>
                    </button>
                  ))}
                </>
              ) : (
                <Empty title={search ? 'No matching keys.' : 'A clean slate.'}>
                  {search
                    ? 'Try a different name or environment.'
                    : resource
                      ? 'Create a key or import a value you already have. Some providers only reveal values once.'
                      : 'Choose an available provider resource to see its keys.'}
                </Empty>
              )}
            </div>
            {credential && (
              <CredentialPanel
                key={credential.id}
                {...{ snapshot, busy, perform }}
                credential={credential}
                manifest={manifest}
                close={() => setSelected('')}
                importValue={() => setModal('import')}
                onRotation={onRotation}
              />
            )}
          </div>
          {modal && (
            <CredentialForm
              mode={modal}
              manifest={manifest}
              connection={connection}
              resource={resource}
              credential={modal === 'import' ? credential : undefined}
              busy={busy}
              perform={perform}
              close={() => setModal(undefined)}
            />
          )}
        </>
      )}
    </>
  );
}

function CredentialForm({
  mode,
  manifest,
  connection,
  resource,
  credential,
  busy,
  perform,
  close,
}: {
  mode: 'import' | 'create';
  manifest: Manifest;
  connection: ConnectionView;
  resource: string;
  credential?: CredentialView;
  busy: boolean;
  perform: Controller['perform'];
  close: () => void;
}) {
  return (
    <Modal
      title={mode === 'import' ? 'Import a credential' : 'Create a credential'}
      onClose={close}
    >
      <p className="quiet">
        {mode === 'import'
          ? 'The value is validated with your provider, then saved to this device.'
          : 'The new value will be saved in your encrypted vault as soon as the provider returns it.'}
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const data = new FormData(e.currentTarget);
          const name = String(data.get('name'));
          void perform(
            () =>
              mode === 'create'
                ? request<CredentialView>({
                    kind: 'create',
                    connection_id: connection.id,
                    resource,
                    name,
                  })
                : request<CredentialView>({
                    kind: 'import',
                    input: {
                      connection_id: connection.id,
                      provider_id: credential?.provider_id ?? null,
                      resource,
                      name,
                      project: String(data.get('project')),
                      environment: String(data.get('environment')),
                      fields: Object.fromEntries(
                        manifest.credential_fields
                          .map((f) => [f.key, String(data.get(f.key) ?? '')])
                          .filter(([, value]) => value !== ''),
                      ),
                    },
                  }),
            'Credential saved',
          ).then((result) => {
            if (result) close();
          });
        }}
      >
        <Field label="Name">
          <input
            name="name"
            defaultValue={credential?.name ?? ''}
            placeholder="Production API key"
            required
            autoFocus
          />
        </Field>
        {mode === 'import' && (
          <>
            <div className="form-row">
              <Field label="Project">
                <input
                  name="project"
                  defaultValue={credential?.project ?? ''}
                  placeholder="my-app"
                />
              </Field>
              <Field label="Environment">
                <input
                  name="environment"
                  defaultValue={credential?.environment || 'development'}
                  required
                />
              </Field>
            </div>
            {manifest.credential_fields.map((f) => (
              <Field key={f.key} label={f.label}>
                <input
                  type="password"
                  name={f.key}
                  required={f.required}
                  placeholder={f.placeholder}
                  autoComplete="new-password"
                  spellCheck={false}
                />
              </Field>
            ))}
          </>
        )}
        <button className="button full" disabled={busy}>
          {mode === 'import' ? 'Validate & save' : 'Create key'}
        </button>
      </form>
    </Modal>
  );
}

function CredentialPanel({
  credential,
  manifest,
  close,
  importValue,
  onRotation,
  ...control
}: Controller & {
  credential: CredentialView;
  manifest: Manifest;
  close: () => void;
  importValue: () => void;
  onRotation: () => void;
}) {
  const { busy, perform, snapshot } = control;
  const [secret, setSecret] = useState<Record<string, string>>();
  const [binding, setBinding] = useState(false);
  const [rotate, setRotate] = useState(false);
  const [guided, setGuided] = useState(false);
  const [guideMessage, setGuideMessage] = useState('');
  useEffect(() => {
    const hide = () => setSecret(undefined);
    const timer = setTimeout(hide, 30000);
    window.addEventListener('blur', hide);
    document.addEventListener('visibilitychange', hide);
    return () => {
      clearTimeout(timer);
      window.removeEventListener('blur', hide);
      document.removeEventListener('visibilitychange', hide);
    };
  }, [secret]);
  const bindings = snapshot.bindings.filter((b) => b.credential_id === credential.id);
  return (
    <aside className="credential-panel">
      <div className="detail-heading">
        <ProviderMark small name={manifest.name} />
        <span>Credential details</span>
        <button className="icon-button" aria-label="Close credential details" onClick={close}>
          <X size={15} />
        </button>
      </div>
      <h2>{credential.name}</h2>
      <span className="small-text quiet">{humanize(credential.credential_type)}</span>
      <dl className="metadata">
        <dt>Provider ID</dt>
        <dd className="mono">{credential.provider_id}</dd>
        <dt>Resource</dt>
        <dd>{credential.resource}</dd>
        <dt>Environment</dt>
        <dd>{credential.environment}</dd>
        <dt>Value</dt>
        <dd>{credential.value_available ? 'Saved in your vault' : 'Not saved on this device'}</dd>
      </dl>
      <div className="secret-box">
        {manifest.credential_fields
          .filter((f) => f.required || secret?.[f.key])
          .map((f) => (
            <div key={f.key}>
              <span className="small-text quiet">{f.label}</span>
              <div className="secret-value">
                <code>{secret?.[f.key] ?? '••••••••••••••••••••'}</code>
                <button
                  className="icon-button"
                  aria-label={`Copy ${f.label}`}
                  disabled={busy}
                  onClick={() =>
                    void perform(
                      () => copyCredential(credential.id, f.key),
                      'Copied. Clipboard clears in 30 seconds.',
                    )
                  }
                >
                  <Copy size={14} />
                </button>
              </div>
            </div>
          ))}
        <button
          className="text-button"
          disabled={busy}
          onClick={() => {
            if (secret) setSecret(undefined);
            else
              void perform(() =>
                request<{ fields: Record<string, string> }>({
                  kind: 'reveal',
                  credential_id: credential.id,
                }),
              ).then((s) => {
                if (s) setSecret(s.fields);
              });
          }}
        >
          {secret ? <EyeOff size={14} /> : <Eye size={14} />}
          {secret ? 'Hide values' : 'Reveal for 30 seconds'}
        </button>
      </div>
      {!credential.value_available && (
        <button className="button secondary full" onClick={importValue}>
          Import this key’s value
        </button>
      )}
      <div className="detail-section">
        <div className="row">
          <h3>
            Consumers <span className="quiet">{bindings.length}</span>
          </h3>
          <button
            className="icon-button"
            title="Add consumer"
            aria-label="Add consumer"
            disabled={!credential.value_available}
            onClick={() => setBinding(true)}
          >
            <Plus size={16} />
          </button>
        </div>
        {bindings.length ? (
          bindings.map((b) => (
            <div className="binding-item" key={b.id}>
              <div>
                <strong className="mono">{b.variable}</strong>
                <small>
                  {b.project} / {b.environment}
                </small>
              </div>
              <button
                className="icon-button"
                disabled={busy}
                aria-label={`Remove ${b.variable}`}
                onClick={() =>
                  void perform(
                    () => request({ kind: 'remove_binding', binding_id: b.id }),
                    'Consumer removed',
                  )
                }
              >
                <X size={13} />
              </button>
            </div>
          ))
        ) : (
          <p className="quiet small-text">
            Add the apps that use this key. GetKeys can update them during rotation.
          </p>
        )}
      </div>
      {(credential.can_rotate || manifest.operations.includes('guide')) && (
        <button
          disabled={
            busy ||
            snapshot.connections.find((c) => c.id === credential.connection_id)?.state ===
              'disconnected'
          }
          className="button secondary full"
          onClick={() => (credential.can_rotate ? setRotate(true) : setGuided(true))}
        >
          <RotateCw size={15} />
          {credential.can_rotate ? 'Rotate this key' : 'Replace in provider'}
          {!credential.can_rotate && <ArrowUpRight size={14} />}
        </button>
      )}
      {binding && (
        <BindingForm
          {...control}
          credential={credential}
          manifest={manifest}
          close={() => setBinding(false)}
        />
      )}
      {rotate && (
        <Modal title="Rotate this key" onClose={() => setRotate(false)}>
          <p>
            GetKeys will create a replacement, update {bindings.length} registered consumer
            {bindings.length === 1 ? '' : 's'}, and wait for your verification before retiring this
            key.
          </p>
          {bindings.length === 0 && (
            <p className="callout">
              No consumers are registered. You will need to update any apps using this key yourself
              before retiring it.
            </p>
          )}
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const name = String(new FormData(e.currentTarget).get('name'));
              void perform(() =>
                request<Rotation>({ kind: 'plan_rotation', credential_id: credential.id, name }),
              ).then((r) => {
                if (r) {
                  setRotate(false);
                  onRotation();
                }
              });
            }}
          >
            <Field label="Replacement name">
              <input name="name" defaultValue={`${credential.name} replacement`} required />
            </Field>
            <button className="button full" disabled={busy}>
              Review rotation plan
            </button>
          </form>
        </Modal>
      )}
      {guided && (
        <Modal title={`Replace your ${manifest.name} token`} onClose={() => setGuided(false)}>
          <p>
            Create a replacement in {manifest.name} with the scopes your apps need, then import it
            as a new key. Return here to select it and update your registered consumers.
          </p>
          <button
            className="button secondary full"
            disabled={busy}
            onClick={() =>
              void perform(() =>
                request<string>({
                  kind: 'guide',
                  connection_id: credential.connection_id,
                  resource: credential.resource,
                }),
              ).then((message) => {
                if (message) setGuideMessage(message);
              })
            }
          >
            Open {manifest.name} <ArrowUpRight size={14} />
          </button>
          {guideMessage && <p className="callout">{guideMessage}</p>}
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const replacementId = String(new FormData(e.currentTarget).get('replacement'));
              void perform(() =>
                request<Rotation>({
                  kind: 'plan_replacement',
                  credential_id: credential.id,
                  replacement_id: replacementId,
                }),
              ).then((r) => {
                if (r) {
                  setGuided(false);
                  onRotation();
                }
              });
            }}
          >
            <Field label="Imported replacement">
              <select name="replacement" required defaultValue="">
                <option value="" disabled>
                  Choose a different saved key
                </option>
                {snapshot.credentials
                  .filter(
                    (c) =>
                      c.id !== credential.id &&
                      c.connection_id === credential.connection_id &&
                      c.resource === credential.resource &&
                      c.value_available &&
                      !c.retired,
                  )
                  .map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
              </select>
            </Field>
            <button className="button full" disabled={busy}>
              Use this replacement
            </button>
          </form>
        </Modal>
      )}
    </aside>
  );
}

function BindingForm({
  credential,
  manifest,
  close,
  snapshot,
  busy,
  perform,
}: Controller & { credential: CredentialView; manifest: Manifest; close: () => void }) {
  const [destination, setDestination] = useState('local');
  const destinations = snapshot.connections.filter(
    (c) =>
      c.state === 'connected' &&
      snapshot.plugins
        .find((p) => p.manifest.id === c.plugin_id)
        ?.manifest.operations.includes('write_secret'),
  );
  return (
    <Modal title="Add a consumer" onClose={close}>
      <p className="quiet">Map a credential field to the environment variable your app expects.</p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const d = new FormData(e.currentTarget);
          const value: Binding = {
            id: '',
            credential_id: credential.id,
            field: String(d.get('field')),
            variable: String(d.get('variable')),
            project: String(d.get('project')),
            environment: String(d.get('environment')),
            destination:
              destination === 'local'
                ? { kind: 'local' }
                : {
                    kind: 'provider',
                    connection_id: destination,
                    resource: String(d.get('resource')),
                  },
          };
          void perform(
            () => request<Binding>({ kind: 'save_binding', binding: value }),
            'Consumer added',
          ).then((b) => {
            if (b) close();
          });
        }}
      >
        <Field label="Credential field">
          <select name="field">
            {manifest.credential_fields.map((f) => (
              <option key={f.key} value={f.key}>
                {f.label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Environment variable">
          <input
            name="variable"
            placeholder="API_TOKEN"
            pattern="[A-Za-z_][A-Za-z0-9_]*"
            required
          />
        </Field>
        <div className="form-row">
          <Field label="Project name">
            <input name="project" placeholder="my-app" defaultValue={credential.project} required />
          </Field>
          <Field label="Environment">
            <input name="environment" defaultValue={credential.environment} required />
          </Field>
        </div>
        <Field label="Destination">
          <select value={destination} onChange={(e) => setDestination(e.target.value)}>
            <option value="local">Local process · getkeys run</option>
            {destinations.map((c) => (
              <option value={c.id} key={c.id}>
                {c.name} ·{' '}
                {snapshot.plugins.find((p) => p.manifest.id === c.plugin_id)?.manifest.name}
              </option>
            ))}
          </select>
        </Field>
        {destination !== 'local' && (
          <Field label="Destination resource ID">
            <input name="resource" placeholder="Project ID or owner/repository" required />
          </Field>
        )}
        <p className="small-text quiet">
          Saving a consumer records the mapping. Remote values are delivered during rotation. GitHub
          destinations use repository Actions secrets; Vercel needs a deployment after delivery.
        </p>
        <button className="button full" disabled={busy}>
          <Check size={15} /> Save consumer
        </button>
      </form>
    </Modal>
  );
}
