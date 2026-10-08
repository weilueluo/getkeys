import { useState } from 'react';
import { ArrowRight, Check, Circle, RotateCw } from 'lucide-react';
import type { Controller } from './controller';
import type { Rotation } from './types';
import { request } from './bridge';
import { Empty, Field, Modal, humanize } from './ui';

const steps = ['Create replacement', 'Update consumers', 'Verify consumers', 'Retire old key'];
function stage(r: Rotation) {
  const s = r.state.kind;
  return s === 'complete'
    ? 4
    : ['ready_to_retire', 'deactivating', 'deactivated', 'retiring'].includes(s)
      ? 3
      : s === 'awaiting_verification'
        ? 2
        : ['stored', 'distributing'].includes(s)
          ? 1
          : 0;
}
export function Rotations({ snapshot, busy, perform }: Controller) {
  const [retire, setRetire] = useState<Rotation>();
  const [verify, setVerify] = useState<{ rotation: Rotation; binding: string }>();
  const [reconcile, setReconcile] = useState<Rotation>();
  const advance = (r: Rotation) =>
    perform(() => request<Rotation>({ kind: 'advance_rotation', rotation_id: r.id }));
  const automaticRetirement = (r: Rotation) =>
    snapshot.plugins
      .find(
        (p) =>
          p.manifest.id === snapshot.connections.find((c) => c.id === r.connection_id)?.plugin_id,
      )
      ?.manifest.operations.includes('revoke_credential') === true;
  return (
    <>
      <div className="page-heading">
        <div className="eyebrow">CHANGE KEYS WITH CONFIDENCE</div>
        <h1>Rotations</h1>
        <p>A replacement first. Verification next. Retirement last.</p>
      </div>
      {snapshot.rotations.length === 0 ? (
        <Empty title="Nothing in rotation.">
          Open a supported key in your vault and choose Rotate this key.
        </Empty>
      ) : (
        <div className="rotation-list">
          {snapshot.rotations.map((r) => {
            const old = snapshot.credentials.find((c) => c.id === r.credential_id);
            const s = r.state.kind;
            const uncertain = ['uncertain', 'creating', 'deactivating', 'retiring'].includes(s);
            return (
              <article key={r.id} className="rotation-card">
                <div className="row">
                  <div>
                    <h2>{old?.name ?? r.name}</h2>
                    <p className="quiet small-text">
                      {r.resource} · {new Date(r.created_at * 1000).toLocaleDateString()}
                    </p>
                  </div>
                  <span className={`badge ${s === 'complete' ? 'success' : ''}`}>
                    {humanize(s)}
                  </span>
                </div>
                <ol className="rotation-steps">
                  {steps.map((step, i) => (
                    <li
                      key={step}
                      className={i < stage(r) ? 'done' : i === stage(r) ? 'current' : ''}
                    >
                      {i < stage(r) ? <Check size={14} /> : <Circle size={14} />}
                      <span>{step}</span>
                    </li>
                  ))}
                </ol>
                {r.bindings.length === 0 && !['complete', 'aborted'].includes(s) && (
                  <p className="callout">
                    No registered consumers. Update and test every app using this key before
                    retiring the old key.
                  </p>
                )}
                {r.bindings.length > 0 && (
                  <div className="rotation-consumers">
                    {r.bindings.map((b) => (
                      <div key={b.id}>
                        <span>
                          <strong>{b.project}</strong>
                          <small>
                            {b.variable} · {b.environment}
                          </small>
                        </span>
                        <span className="small-text quiet">
                          {r.verified_bindings.includes(b.id)
                            ? 'Verified'
                            : r.updated_bindings.includes(b.id)
                              ? 'Updated · verify deployment'
                              : 'Waiting'}
                        </span>
                        {s === 'awaiting_verification' && !r.verified_bindings.includes(b.id) && (
                          <button
                            className="button secondary compact"
                            disabled={busy}
                            onClick={() => setVerify({ rotation: r, binding: b.id })}
                          >
                            Verify
                          </button>
                        )}
                      </div>
                    ))}
                  </div>
                )}
                <div className="rotation-actions">
                  {s === 'planned' && (
                    <>
                      <button
                        className="text-button quiet"
                        disabled={busy}
                        onClick={() =>
                          void perform(() => request({ kind: 'abort_rotation', rotation_id: r.id }))
                        }
                      >
                        Cancel plan
                      </button>
                      <button
                        className="button compact"
                        disabled={busy}
                        onClick={() => void advance(r)}
                      >
                        Create replacement <ArrowRight size={14} />
                      </button>
                    </>
                  )}
                  {['stored', 'distributing'].includes(s) && (
                    <button
                      className="button compact"
                      disabled={busy}
                      onClick={() => void advance(r)}
                    >
                      Validate & update consumers <ArrowRight size={14} />
                    </button>
                  )}
                  {['ready_to_retire', 'deactivated'].includes(s) && (
                    <button className="button compact" disabled={busy} onClick={() => setRetire(r)}>
                      {s === 'deactivated' ? 'Delete old key' : 'Retire old key'}{' '}
                      <ArrowRight size={14} />
                    </button>
                  )}
                  {uncertain && (
                    <>
                      <p className="small-text">
                        A provider write was not confirmed. Inspect the provider before continuing.
                      </p>
                      <button
                        className="button secondary compact"
                        disabled={busy}
                        onClick={() =>
                          ['deactivating', 'retiring'].includes(s) ||
                          (r.state.kind === 'uncertain' && r.state.action !== 'create')
                            ? void perform(() =>
                                request({ kind: 'reconcile_retirement', rotation_id: r.id }),
                              )
                            : setReconcile(r)
                        }
                      >
                        Reconcile outcome
                      </button>
                    </>
                  )}
                </div>
              </article>
            );
          })}
        </div>
      )}
      {retire && (
        <Modal
          title={
            retire.state.kind === 'deactivated' ? 'Delete the old key?' : 'Retire the old key?'
          }
          onClose={() => setRetire(undefined)}
        >
          <p>
            Confirm that every app using this key has been updated and tested with the replacement.
            Apps still using the old key will lose access.
          </p>
          <p className="quiet">
            {automaticRetirement(retire)
              ? 'AWS keys are deactivated first. Deleting a key is permanent.'
              : 'Revoke the OLD token in your provider dashboard now. GetKeys cannot revoke this token through the API. Confirm only after provider-side revocation is complete.'}
          </p>
          {!automaticRetirement(retire) && (
            <button
              className="button secondary full"
              disabled={busy}
              onClick={() =>
                void perform(() =>
                  request({
                    kind: 'guide',
                    connection_id: retire.connection_id,
                    resource: retire.resource,
                  }),
                )
              }
            >
              Open provider dashboard
            </button>
          )}
          <button
            className="button full"
            disabled={busy}
            onClick={() =>
              void (
                automaticRetirement(retire)
                  ? advance(retire)
                  : perform(() =>
                      request<Rotation>({
                        kind: 'confirm_manual_retirement',
                        rotation_id: retire.id,
                      }),
                    )
              ).then((r) => {
                if (r) setRetire(undefined);
              })
            }
          >
            {automaticRetirement(retire)
              ? 'I verified my apps — continue'
              : 'I revoked the old token — complete rotation'}
          </button>
        </Modal>
      )}
      {verify && (
        <Modal title="Verify this consumer" onClose={() => setVerify(undefined)}>
          <p>
            Deploy or restart the app with the replacement credential, then test a request that
            needs it. A successful secret write alone does not verify the deployment.
          </p>
          <button
            className="button full"
            disabled={busy}
            onClick={() =>
              void perform(() =>
                request({
                  kind: 'verify_binding',
                  rotation_id: verify.rotation.id,
                  binding_id: verify.binding,
                }),
              ).then((r) => {
                if (r) setVerify(undefined);
              })
            }
          >
            <Check size={15} /> The app works with the new key
          </button>
        </Modal>
      )}
      {reconcile && (
        <Modal title="Find the replacement" onClose={() => setReconcile(undefined)}>
          <p>
            Inspect the provider for the key created by this attempt. If its value is only shown
            once, import it in My keys before continuing.
          </p>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const id = String(new FormData(e.currentTarget).get('provider_id'));
              void perform(() =>
                request({ kind: 'reconcile_rotation', rotation_id: reconcile.id, provider_id: id }),
              ).then((r) => {
                if (r) setReconcile(undefined);
              });
            }}
          >
            <Field label="Replacement provider ID">
              <input name="provider_id" required />
            </Field>
            <button className="button full" disabled={busy}>
              <RotateCw size={14} /> Reconcile replacement
            </button>
          </form>
        </Modal>
      )}
    </>
  );
}
