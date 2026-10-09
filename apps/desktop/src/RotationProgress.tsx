import { useState } from 'react';
import { ArrowRight, Check, RotateCw } from 'lucide-react';
import type { Controller } from './controller';
import type { Rotation, Snapshot } from './types';
import { request } from './bridge';
import { Field, Modal, sentence } from './ui';

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
export const isFinished = (r: Rotation) => ['complete', 'aborted'].includes(r.state.kind);
/** Rotations that replace this key or produced it as the replacement, newest first. */
export const rotationsFor = (snapshot: Snapshot, credentialId: string) =>
  snapshot.rotations
    .filter((r) => r.credential_id === credentialId || r.candidate_id === credentialId)
    .sort((a, b) => b.created_at - a.created_at);
export const formatDay = (seconds: number) => new Date(seconds * 1000).toLocaleDateString();

export function RotationProgress({
  rotation: r,
  snapshot,
  busy,
  perform,
}: Controller & { rotation: Rotation }) {
  const [retire, setRetire] = useState(false);
  const [verify, setVerify] = useState<string>();
  const [reconcile, setReconcile] = useState(false);
  const advance = () =>
    perform(() => request<Rotation>({ kind: 'advance_rotation', rotation_id: r.id }));
  const automaticRetirement =
    snapshot.plugins
      .find(
        (p) =>
          p.manifest.id === snapshot.connections.find((c) => c.id === r.connection_id)?.plugin_id,
      )
      ?.manifest.operations.includes('revoke_credential') === true;
  const s = r.state.kind;
  const uncertain = ['uncertain', 'creating', 'deactivating', 'retiring'].includes(s);
  const current = stage(r);
  return (
    <section className="detail-section rotation" aria-label="Rotation">
      <div className="detail-section-heading">
        <h3>Rotation</h3>
        <span className={`state ${uncertain ? 'warning' : 'active'}`}>{sentence(s)}</span>
      </div>
      <p className="quiet">
        {r.name} · started {formatDay(r.created_at)}
      </p>
      <ol className="rotation-steps" aria-label="Rotation progress">
        {steps.map((step, i) => (
          <li
            key={step}
            className={i < current ? 'done' : i === current ? 'current' : ''}
            aria-current={i === current ? 'step' : undefined}
          >
            <span className="step-number mono">
              {i < current ? <Check size={11} strokeWidth={2.5} /> : i + 1}
            </span>
            <span>{step}</span>
          </li>
        ))}
      </ol>
      {r.bindings.length === 0 ? (
        <p className="callout">
          No apps are registered for this key. Update and test every app that uses it before
          retiring the old key.
        </p>
      ) : (
        <div className="rotation-consumers">
          {r.bindings.map((b) => (
            <div key={b.id}>
              <span>
                <strong className="mono">{b.variable}</strong>
                <small>
                  {b.project} · {b.environment}
                </small>
              </span>
              <span
                className={`consumer-state ${r.verified_bindings.includes(b.id) ? 'verified' : ''}`}
              >
                {r.verified_bindings.includes(b.id)
                  ? 'Verified'
                  : r.updated_bindings.includes(b.id)
                    ? 'Updated · verify deployment'
                    : 'Waiting'}
              </span>
              {s === 'awaiting_verification' && !r.verified_bindings.includes(b.id) && (
                <button
                  className="button secondary small"
                  disabled={busy}
                  onClick={() => setVerify(b.id)}
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
              className="text-button"
              disabled={busy}
              onClick={() =>
                void perform(() => request({ kind: 'abort_rotation', rotation_id: r.id }))
              }
            >
              Cancel plan
            </button>
            <button className="button" disabled={busy} onClick={() => void advance()}>
              Create replacement <ArrowRight size={14} />
            </button>
          </>
        )}
        {['stored', 'distributing'].includes(s) && (
          <button className="button" disabled={busy} onClick={() => void advance()}>
            Validate & update consumers <ArrowRight size={14} />
          </button>
        )}
        {['ready_to_retire', 'deactivated'].includes(s) && (
          <button className="button" disabled={busy} onClick={() => setRetire(true)}>
            {s === 'deactivated' ? 'Delete old key' : 'Retire old key'} <ArrowRight size={14} />
          </button>
        )}
        {uncertain && (
          <>
            <p className="warning-text">
              A provider change wasn’t confirmed. Check the provider before continuing.
            </p>
            <button
              className="button secondary"
              disabled={busy}
              onClick={() =>
                ['deactivating', 'retiring'].includes(s) ||
                (r.state.kind === 'uncertain' && r.state.action !== 'create')
                  ? void perform(() => request({ kind: 'reconcile_retirement', rotation_id: r.id }))
                  : setReconcile(true)
              }
            >
              Reconcile outcome
            </button>
          </>
        )}
      </div>
      {retire && (
        <Modal
          title={s === 'deactivated' ? 'Delete the old key?' : 'Retire the old key?'}
          onClose={() => setRetire(false)}
        >
          <p>
            Confirm that every app using this key has been updated and tested with the replacement.
            Apps still using the old key will lose access.
          </p>
          <p className="quiet">
            {automaticRetirement
              ? 'AWS keys are deactivated first. Deleting a key is permanent.'
              : 'Revoke the OLD token in your provider dashboard now. GetKeys cannot revoke this token through the API. Confirm only after provider-side revocation is complete.'}
          </p>
          {!automaticRetirement && (
            <button
              className="button secondary full"
              disabled={busy}
              onClick={() =>
                void perform(() =>
                  request({ kind: 'guide', connection_id: r.connection_id, resource: r.resource }),
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
                automaticRetirement
                  ? advance()
                  : perform(() =>
                      request<Rotation>({ kind: 'confirm_manual_retirement', rotation_id: r.id }),
                    )
              ).then((done) => {
                if (done) setRetire(false);
              })
            }
          >
            {automaticRetirement
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
                request({ kind: 'verify_binding', rotation_id: r.id, binding_id: verify }),
              ).then((done) => {
                if (done) setVerify(undefined);
              })
            }
          >
            <Check size={15} /> The app works with the new key
          </button>
        </Modal>
      )}
      {reconcile && (
        <Modal title="Find the replacement" onClose={() => setReconcile(false)}>
          <p>
            Inspect the provider for the key created by this attempt. If its value is only shown
            once, import it in My keys before continuing.
          </p>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const id = String(new FormData(e.currentTarget).get('provider_id'));
              void perform(() =>
                request({ kind: 'reconcile_rotation', rotation_id: r.id, provider_id: id }),
              ).then((done) => {
                if (done) setReconcile(false);
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
    </section>
  );
}
