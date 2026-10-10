import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ArrowUpRight, ChevronRight, Lock, LockOpen, Plus, X } from 'lucide-react';
import type {
  Created,
  CredentialView,
  Manifest,
  NewResource,
  Owner,
  PluginView,
  Resource,
} from './types';
import type { Controller } from './controller';
import { cancelCapture, captureKey, request } from './bridge';
import { Field, inputs, Modal, ProviderMark, sentence } from './ui';

/** How often and how long to wait for a new project to start. */
export const waiting = { every: 5000, tries: 48 };
const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));

/** A name providers accept for a project: lowercase letters, numbers, and single hyphens. */
export const slug = (value: string) =>
  value
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^[-._]+/, '')
    .slice(0, 100);

/** The account choice for a key kept on its own, outside any account. */
const OWN = 'own';
type Way = 'create' | 'capture' | 'paste';

/** One step of the cascade: its choices while open, a one-line summary once chosen. */
function Step({
  label,
  chosen,
  change,
  children,
}: {
  label: string;
  chosen?: ReactNode;
  change?: () => void;
  children?: ReactNode;
}) {
  if (chosen !== undefined)
    return (
      <li className="cascade-step done">
        <span className="cascade-label">{label}</span>
        <span className="cascade-value">{chosen}</span>
        {change && (
          <button
            type="button"
            className="text-button"
            aria-label={`Change ${label.toLowerCase()}`}
            onClick={change}
          >
            Change
          </button>
        )}
      </li>
    );
  return (
    <li className="cascade-step">
      <span className="cascade-label">{label}</span>
      <div className="cascade-body" role="group" aria-label={label}>
        {children}
      </div>
    </li>
  );
}

function Choice({
  title,
  note,
  mark,
  icon,
  disabled,
  onClick,
}: {
  title: string;
  note?: string;
  mark?: string;
  icon?: ReactNode;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button type="button" className="cascade-choice" disabled={disabled} onClick={onClick}>
      {mark !== undefined ? <ProviderMark small name={mark} /> : icon}
      <span className="choice-text">
        <strong>{title}</strong>
        {note && <small>{note}</small>}
      </span>
      <ChevronRight size={15} strokeWidth={1.75} />
    </button>
  );
}

/** The provider's page for making a key by hand, if it has one. */
export const keyPage = (m: Manifest) => m.key_page || m.token_page;

/**
 * Makes a key on the provider's page and picks it up from the clipboard: GetKeys opens the
 * page filled in, the person clicks Create and Copy there, and the key is saved without passing
 * through this window.
 */
export function CaptureKey({
  manifest,
  connectionId,
  resource,
  name,
  group,
  replaces,
  busy,
  perform,
  done,
}: Pick<Controller, 'busy' | 'perform'> & {
  manifest: Manifest;
  connectionId?: string;
  resource?: string;
  name: string;
  group?: string;
  /** The key the new one replaces; the new one joins its groups. */
  replaces?: string;
  done: (key: CredentialView) => void;
}) {
  const [waiting, setWaiting] = useState(false);
  const live = useRef(false);
  useEffect(
    () => () => {
      if (live.current) void cancelCapture().catch(() => {});
    },
    [],
  );
  const start = () => {
    setWaiting(true);
    live.current = true;
    void perform(
      () =>
        captureKey({
          pluginId: manifest.id,
          connectionId: connectionId ?? null,
          resource: resource ?? '',
          name: name.trim(),
          group: group ?? null,
          replaces: replaces ?? null,
        }),
      (key) => (key ? `Saved ${key.name}` : undefined),
    ).then((key) => {
      live.current = false;
      setWaiting(false);
      if (key) done(key);
    });
  };
  return waiting ? (
    <div className="capture">
      <p className="waiting" role="status">
        <span className="pulse" aria-hidden="true" />
        Waiting for the new key
      </p>
      <p className="small-text quiet">
        In {manifest.name}, create the key, then copy it. GetKeys picks it up from the clipboard,
        checks it and keeps it encrypted on this device.
      </p>
      <button
        type="button"
        className="button outline small"
        onClick={() => void cancelCapture().catch(() => {})}
      >
        Cancel
      </button>
    </div>
  ) : (
    <div className="capture">
      <p className="small-text quiet">
        {manifest.name} makes keys only on its own page. GetKeys opens it filled in; create the key
        there and copy it, and GetKeys keeps it. No pasting needed.
      </p>
      <button type="button" className="button full" disabled={busy || !name.trim()} onClick={start}>
        Make it on {manifest.name} <ArrowUpRight size={14} />
      </button>
    </div>
  );
}

/**
 * Makes a new key one choice at a time: the provider, then the account, then where the key
 * belongs (or a new project for it). The provider makes the key; providers that can't make
 * keys through their API make it on their page, and GetKeys picks it up. Pasting a key is
 * there for keys made elsewhere.
 */
export function NewKeyDialog({
  snapshot,
  busy,
  perform,
  group,
  browse,
  connect,
  close,
}: Controller & {
  /** The group being viewed; the new key joins it. */
  group?: string;
  /** Shows the providers page. */
  browse: () => void;
  /** Connects an account with a provider. */
  connect: (plugin: PluginView) => void;
  close: () => void;
}) {
  const [provider, setProvider] = useState<string>();
  const [find, setFind] = useState('');
  const [account, setAccount] = useState<string>();
  const [resources, setResources] = useState<Resource[] | 'failed'>();
  const [place, setPlace] = useState<string>();
  const [making, setMaking] = useState(false);
  const [pasting, setPasting] = useState(false);
  const [name, setName] = useState('');

  const providers = snapshot.plugins
    .filter((p) => p.installed)
    .sort((a, b) => a.manifest.name.localeCompare(b.manifest.name));
  const plugin = providers.find((p) => p.manifest.id === provider);
  const manifest = plugin?.manifest;
  const accounts = snapshot.connections.filter(
    (c) => c.plugin_id === provider && c.state !== 'disconnected',
  );
  const pastes = !!manifest && inputs(manifest.credential_fields).length > 0;
  const declarative = manifest?.kind === 'declarative';
  const page = manifest ? keyPage(manifest) : '';
  // Keys of declarative providers are never under an account; one account is picked by itself.
  const accountId = declarative
    ? OWN
    : (account ?? (accounts.length === 1 ? accounts[0].id : undefined));
  const connection = accounts.find((c) => c.id === accountId);
  const makesKeys = !!connection && !!manifest?.operations.includes('create_credential');
  const makesProjects =
    !!connection && !!manifest?.new_resource && manifest.operations.includes('create_resource');
  const needsPlace =
    !!connection &&
    !manifest?.account_credentials &&
    !!manifest?.operations.includes('list_resources');
  const listed = Array.isArray(resources) ? resources : undefined;
  const placeId =
    !needsPlace || !connection
      ? ''
      : (place ??
        (listed && !makesProjects && listed.length <= 1 ? (listed[0]?.id ?? '') : undefined));
  const ready = !!manifest && accountId !== undefined && placeId !== undefined;
  const way: Way | undefined = !ready
    ? undefined
    : pasting && pastes
      ? 'paste'
      : makesKeys
        ? 'create'
        : pastes && page
          ? 'capture'
          : pastes
            ? 'paste'
            : undefined;
  const noun = manifest?.new_resource?.noun ?? 'project';

  useEffect(() => {
    if (!connection || !needsPlace) return;
    let current = true;
    setResources(undefined);
    void perform(() =>
      request<Resource[]>({ kind: 'resources', connection_id: connection.id }),
    ).then((r) => {
      if (current) setResources(r ?? 'failed');
    });
    return () => {
      current = false;
    };
  }, [connection?.id, needsPlace, perform]);

  const pickProvider = (id?: string) => {
    setProvider(id);
    setAccount(undefined);
    setPlace(undefined);
    setMaking(false);
    setPasting(false);
  };
  const pickAccount = (id?: string) => {
    setAccount(id);
    setPlace(undefined);
    setMaking(false);
    setPasting(false);
  };
  const pickPlace = (id?: string) => {
    setPlace(id);
    setMaking(false);
  };
  /** Puts a new key in the group being viewed. */
  const file = async (key: CredentialView) =>
    group
      ? request<CredentialView>({ kind: 'set_groups', credential_id: key.id, groups: [group] })
      : key;

  const providerStep = (
    <Step
      label="Provider"
      chosen={
        provider === CUSTOM_PROVIDER ? (
          <span className="cascade-chosen">
            <ProviderMark small name="Custom" /> Any other service
          </span>
        ) : (
          manifest && (
            <span className="cascade-chosen">
              <ProviderMark small name={manifest.name} /> {manifest.name}
            </span>
          )
        )
      }
      change={() => pickProvider(undefined)}
    >
      {providers.length > 6 && (
        <input
          className="cascade-find"
          aria-label="Find a provider"
          placeholder="Find a provider"
          value={find}
          autoFocus
          onChange={(e) => setFind(e.target.value)}
        />
      )}
      <div className="choices">
        {providers
          .filter((p) => p.manifest.name.toLowerCase().includes(find.trim().toLowerCase()))
          .map((p) => (
            <Choice
              key={p.manifest.id}
              mark={p.manifest.name}
              title={p.manifest.name}
              onClick={() => pickProvider(p.manifest.id)}
            />
          ))}
        <Choice
          mark="Custom"
          title="Any other service"
          note="Name the values yourself"
          onClick={() => pickProvider(CUSTOM_PROVIDER)}
        />
      </div>
      <button type="button" className="button ghost small" onClick={browse}>
        Add a provider <ArrowUpRight size={13} />
      </button>
    </Step>
  );

  if (provider === CUSTOM_PROVIDER)
    return (
      <CustomKeyForm
        {...{ snapshot, busy, perform, close, group }}
        header={<ol className="cascade">{providerStep}</ol>}
      />
    );

  const accountName = (id: string) =>
    id === OWN ? 'None' : (accounts.find((c) => c.id === id)?.name ?? id);
  const placeName = (id: string) =>
    listed?.find((r) => r.id === id)?.name || (id ? id : `No ${noun}`);
  const site = manifest ? new URL(page || manifest.homepage).hostname.replace(/^www\./, '') : '';

  return (
    <Modal
      title="New key"
      description={group ? `It joins ${group}.` : undefined}
      onClose={close}
      wide
    >
      <ol className="cascade">
        {providerStep}
        {manifest && !declarative && (
          <Step
            label="Account"
            chosen={accountId === undefined ? undefined : accountName(accountId)}
            change={account !== undefined ? () => pickAccount(undefined) : undefined}
          >
            <div className="choices">
              {accounts.map((c) => (
                <Choice
                  key={c.id}
                  mark={manifest.name}
                  title={c.name}
                  note={
                    manifest.operations.includes('create_credential')
                      ? `${manifest.name} makes the key`
                      : undefined
                  }
                  onClick={() => pickAccount(c.id)}
                />
              ))}
              {plugin && (
                <Choice
                  icon={
                    <span className="choice-icon" aria-hidden="true">
                      <Plus size={14} />
                    </span>
                  }
                  title={accounts.length ? 'Another account' : `Connect a ${manifest.name} account`}
                  note={`So ${manifest.name} can make keys for you`}
                  onClick={() => {
                    close();
                    connect(plugin);
                  }}
                />
              )}
            </div>
            {pastes && page && !accounts.length && (
              <button type="button" className="text-button" onClick={() => pickAccount(OWN)}>
                Make it on {site} instead
              </button>
            )}
          </Step>
        )}
        {connection && needsPlace && (
          <Step
            label={sentence(listed?.[0]?.kind || noun).replace(/^Iam\b/, 'IAM')}
            chosen={placeId === undefined ? undefined : placeName(placeId)}
            change={
              makesProjects || (listed?.length ?? 0) > 1 ? () => pickPlace(undefined) : undefined
            }
          >
            {resources === undefined ? (
              <p className="quiet">Loading…</p>
            ) : resources === 'failed' ? (
              <p className="quiet">Couldn’t list them. Close and try again.</p>
            ) : making ? (
              <NewProject
                {...{ busy, perform }}
                connectionId={connection.id}
                manifestName={manifest!.name}
                newResource={manifest!.new_resource!}
                cancel={() => setMaking(false)}
                made={(all, id) => {
                  setResources(all);
                  if (id) pickPlace(id);
                  else setMaking(false);
                }}
              />
            ) : (
              <div className="choices">
                {listed!.map((r) => (
                  <Choice
                    key={r.id}
                    title={r.name || r.id}
                    note={r.pending ? r.note || 'Still starting' : r.name ? r.id : undefined}
                    disabled={r.pending}
                    icon={
                      <span className="choice-icon" aria-hidden="true">
                        {(r.name || r.id).slice(0, 1)}
                      </span>
                    }
                    onClick={() => pickPlace(r.id)}
                  />
                ))}
                {makesProjects && (
                  <Choice
                    icon={
                      <span className="choice-icon" aria-hidden="true">
                        <Plus size={14} />
                      </span>
                    }
                    title={`New ${noun}`}
                    note={`For a key in a ${noun} that doesn’t exist yet`}
                    onClick={() => setMaking(true)}
                  />
                )}
              </div>
            )}
          </Step>
        )}
      </ol>
      {manifest && ready && !way && (
        <p className="quiet">{manifest.name} keys can’t be added here yet.</p>
      )}
      {manifest && way && (
        <form
          className="cascade-form"
          onSubmit={(e) => {
            e.preventDefault();
            if (way === 'capture') return;
            const data = new FormData(e.currentTarget);
            const fields = Object.fromEntries(
              inputs(manifest.credential_fields)
                .map((f) => [f.key, String(data.get(f.key) ?? '').trim()])
                .filter(([, value]) => value !== ''),
            );
            const key = name.trim();
            void perform(
              async () =>
                file(
                  way === 'create'
                    ? await request<CredentialView>({
                        kind: 'create',
                        connection_id: accountId!,
                        resource: placeId!,
                        name: key,
                      })
                    : accountId === OWN
                      ? await request<CredentialView>({
                          kind: 'save_key',
                          credential_id: null,
                          plugin_id: manifest.id,
                          name: key,
                          fields,
                        })
                      : await request<CredentialView>({
                          kind: 'import',
                          input: {
                            connection_id: accountId!,
                            provider_id: null,
                            resource: placeId!,
                            name: key,
                            project: '',
                            environment: '',
                            fields,
                          },
                        }),
                ),
              way === 'create' ? `Created ${key}` : 'Key saved',
            ).then((result) => {
              if (result) close();
            });
          }}
        >
          <Field label="Name">
            <input
              name="name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Production API key"
              required
              autoFocus
            />
          </Field>
          {way === 'paste' &&
            inputs(manifest.credential_fields).map((f) => (
              <Field key={f.key} label={f.required ? f.label : `${f.label} (optional)`}>
                <input
                  type={f.secret ? 'password' : 'text'}
                  name={f.key}
                  required={f.required}
                  placeholder={f.placeholder}
                  autoComplete="new-password"
                  spellCheck={false}
                />
              </Field>
            ))}
          {way === 'capture' ? (
            <CaptureKey
              {...{ manifest, busy, perform, name, group }}
              connectionId={accountId === OWN ? undefined : accountId}
              resource={placeId}
              done={close}
            />
          ) : (
            <>
              <p className="small-text quiet">
                {way === 'create'
                  ? `${manifest.name} makes a new key; GetKeys keeps it encrypted on this device.`
                  : accountId === OWN
                    ? 'GetKeys keeps it encrypted on this device.'
                    : `GetKeys checks it with ${manifest.name}, then keeps it encrypted on this device.`}
              </p>
              <button className="button full" disabled={busy}>
                {way === 'create' ? 'Create key' : 'Save key'}
              </button>
            </>
          )}
          {pastes && way !== 'paste' && (
            <button type="button" className="text-button" onClick={() => setPasting(true)}>
              Already have a key? Paste it
            </button>
          )}
          {pasting && (makesKeys || !!page) && (
            <button type="button" className="text-button" onClick={() => setPasting(false)}>
              {makesKeys ? `Have ${manifest.name} make one instead` : 'Make a new one instead'}
            </button>
          )}
        </form>
      )}
    </Modal>
  );
}

const CUSTOM_PROVIDER = 'custom';

/** Makes a project for the new key, and waits until it can hold keys. */
function NewProject({
  connectionId,
  manifestName,
  newResource,
  cancel,
  made,
  busy,
  perform,
}: Pick<Controller, 'busy' | 'perform'> & {
  connectionId: string;
  manifestName: string;
  newResource: NewResource;
  cancel: () => void;
  /** The account's projects now, and the new one once it can hold keys. */
  made: (all: Resource[], id?: string) => void;
}) {
  const [owners, setOwners] = useState<Owner[]>();
  const [name, setName] = useState('');
  const [starting, setStarting] = useState('');
  useEffect(() => {
    let current = true;
    void perform(() => request<Owner[]>({ kind: 'owners', connection_id: connectionId })).then(
      (o) => {
        if (current) setOwners(o ?? []);
      },
    );
    return () => {
      current = false;
    };
  }, [connectionId, perform]);
  const project = slug(name).replace(/[-._]+$/, '');
  return (
    <form
      className="new-project"
      aria-label={`New ${newResource.noun}`}
      onSubmit={(e) => {
        e.preventDefault();
        const data = new FormData(e.currentTarget);
        const options = Object.fromEntries(
          newResource.options.map((o) => [o.key, String(data.get(o.key) ?? o.default)]),
        );
        let saved = '';
        void perform(
          async () => {
            const c = await request<Created>({
              kind: 'create_resource',
              connection_id: connectionId,
              owner: String(data.get('owner') ?? owners?.[0]?.id ?? ''),
              name: project,
              options,
            });
            saved = c.generated?.name ?? '';
            let all: Resource[] = [c.resource];
            for (let i = 0; c.resource.pending; i++) {
              if (i >= waiting.tries) {
                made(all);
                throw `${project} is still starting. Try New key again in a few minutes.`;
              }
              setStarting(`Starting ${project}. This takes a minute or two.`);
              await sleep(waiting.every);
              all = await request<Resource[]>({ kind: 'resources', connection_id: connectionId });
              if (all.some((r) => r.id === c.resource.id && !r.pending)) break;
            }
            if (!all.some((r) => r.id === c.resource.id)) all = [...all, c.resource];
            made(all, c.resource.id);
            return c;
          },
          () =>
            saved ? `Created ${project}. Its ${saved} is saved as a key.` : `Created ${project}`,
        ).finally(() => setStarting(''));
      }}
    >
      <Field label={`${sentence(newResource.noun)} name`}>
        <input
          value={name}
          onChange={(e) => setName(slug(e.target.value))}
          placeholder="my-app"
          required
          autoFocus
          spellCheck={false}
        />
      </Field>
      {owners && owners.length > 1 && (
        <Field label="Organization">
          <select name="owner" defaultValue={owners[0].id}>
            {owners.map((o) => (
              <option key={o.id} value={o.id}>
                {o.name}
              </option>
            ))}
          </select>
        </Field>
      )}
      {newResource.options.map((o) => (
        <Field key={o.key} label={o.label}>
          <select name={o.key} defaultValue={o.default}>
            {o.choices.map((c) => (
              <option key={c.value} value={c.value}>
                {c.label}
              </option>
            ))}
          </select>
        </Field>
      ))}
      {starting && (
        <p className="small-text quiet" role="status">
          {starting}
        </p>
      )}
      {owners && owners.length === 0 && (
        <p className="small-text quiet">
          This {manifestName} account has no organization to create it in.
        </p>
      )}
      <div className="button-row">
        <button type="button" className="button outline small" disabled={busy} onClick={cancel}>
          Back
        </button>
        <button className="button small" disabled={busy || !project || !owners?.length}>
          Create {newResource.noun}
        </button>
      </div>
    </form>
  );
}

type Row = { variable: string; value: string; secret: boolean };
const variableName = (s: string) => s.toUpperCase().replace(/[^A-Z0-9_]/g, '_');

/** A key from any service, saved as named values. Editing keeps secrets left blank. */
export function CustomKeyForm({
  credential,
  header,
  group,
  close,
  busy,
  perform,
}: Controller & {
  credential?: CredentialView;
  header?: ReactNode;
  group?: string;
  close: () => void;
}) {
  const [rows, setRows] = useState<Row[]>(
    credential?.fields.map((f) => ({
      variable: f.key,
      value: f.secret ? '' : (f.value ?? ''),
      secret: f.secret,
    })) ?? [{ variable: '', value: '', secret: true }],
  );
  const kept = (r: Row) =>
    r.secret && !!credential?.fields.some((f) => f.secret && f.key === r.variable);
  const update = (i: number, patch: Partial<Row>) =>
    setRows((all) => all.map((r, j) => (i === j ? { ...r, ...patch } : r)));
  /** Pasting `NAME=value` lines fills in a row for each. */
  const paste = (i: number, text: string) => {
    const parsed = text
      .split(/\r?\n/)
      .map((l) => /^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$/.exec(l))
      .flatMap((m) =>
        m
          ? [
              {
                variable: m[1].toUpperCase(),
                value: m[2].replace(/^(['"])(.*)\1$/, '$2'),
                secret: true,
              },
            ]
          : [],
      );
    if (!parsed.length) return false;
    setRows((all) => [...all.slice(0, i), ...parsed, ...all.slice(i + 1)]);
    return true;
  };
  return (
    <Modal
      title={credential ? `Replace ${credential.name}` : 'New key'}
      description={!credential && group ? `It joins ${group}.` : undefined}
      onClose={close}
      wide={!credential}
    >
      {header}
      <p>
        {credential
          ? 'Paste the new values. Secret values you leave blank stay as they are.'
          : 'Any key from any service. Each value is exported under its variable name. Paste NAME=value lines to fill in several at once.'}
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const name = String(new FormData(e.currentTarget).get('name'));
          void perform(async () => {
            const saved = await request<CredentialView>({
              kind: 'save_custom',
              credential_id: credential?.id ?? null,
              name,
              fields: rows,
            });
            return !credential && group
              ? request<CredentialView>({
                  kind: 'set_groups',
                  credential_id: saved.id,
                  groups: [group],
                })
              : saved;
          }, 'Key saved').then((result) => {
            if (result) close();
          });
        }}
      >
        <Field label="Name">
          <input
            name="name"
            defaultValue={credential?.name ?? ''}
            placeholder="OpenAI"
            required
            autoFocus
          />
        </Field>
        <div className="field">
          <span>Values</span>
          <div className="value-rows">
            {rows.map((r, i) => (
              <div className="value-row" key={i}>
                <input
                  aria-label={`Variable ${i + 1}`}
                  className="mono"
                  placeholder="API_KEY"
                  value={r.variable}
                  required
                  spellCheck={false}
                  onChange={(e) => update(i, { variable: variableName(e.target.value) })}
                  onPaste={(e) => {
                    if (paste(i, e.clipboardData.getData('text'))) e.preventDefault();
                  }}
                />
                <input
                  aria-label={`Value of ${r.variable || `variable ${i + 1}`}`}
                  type={r.secret ? 'password' : 'text'}
                  placeholder={kept(r) ? 'Unchanged' : 'Value'}
                  value={r.value}
                  required={!kept(r)}
                  autoComplete="new-password"
                  spellCheck={false}
                  onChange={(e) => update(i, { value: e.target.value })}
                  onPaste={(e) => {
                    if (paste(i, e.clipboardData.getData('text'))) e.preventDefault();
                  }}
                />
                <button
                  type="button"
                  className="icon-button outline"
                  aria-pressed={r.secret}
                  aria-label={`${r.variable || 'Value'} is secret`}
                  title={r.secret ? 'Secret: hidden until revealed' : 'Not secret: always shown'}
                  onClick={() => update(i, { secret: !r.secret })}
                >
                  {r.secret ? <Lock size={14} /> : <LockOpen size={14} />}
                </button>
                <button
                  type="button"
                  className="icon-button"
                  aria-label={`Remove ${r.variable || `variable ${i + 1}`}`}
                  disabled={rows.length === 1}
                  onClick={() => setRows((all) => all.filter((_, j) => j !== i))}
                >
                  <X size={14} />
                </button>
              </div>
            ))}
          </div>
          <button
            type="button"
            className="button ghost small add-row"
            onClick={() => setRows((all) => [...all, { variable: '', value: '', secret: true }])}
          >
            <Plus size={13} /> Add value
          </button>
        </div>
        <button className="button full" disabled={busy}>
          Save key
        </button>
      </form>
    </Modal>
  );
}
