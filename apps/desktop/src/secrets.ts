import { useEffect, useState } from 'react';
import type { ConnectionView, CredentialView, KeyField } from './types';
import type { Controller } from './controller';
import { request } from './bridge';

/** Credential type of the key an account signs GetKeys in with. */
export const SIGN_IN = 'sign_in';
/** Credential type of a key added by hand that belongs to no provider. */
export const CUSTOM = 'custom';
/** The group that collects keys a provider refused: no permission to list them, or rejected. */
export const FAILED = 'Failed';
/** Whether an account signs in with a key found on this device (a CLI, profile, or variable),
 * rather than one pasted in or created for GetKeys. Such keys are listed under Keys. */
export const borrowed = (source: string | null) =>
  !!source && source !== 'token' && source !== 'getkeys';
/** Whether Keys lists a key: every live key except management keys that weren't found here. */
export const listed = (c: CredentialView, connections: ConnectionView[]) =>
  !c.retired &&
  (c.credential_type !== SIGN_IN ||
    borrowed(connections.find((x) => x.id === c.connection_id)?.source ?? null));
/** Field name that copies every exported field as `.env` lines (see `App::clip`). */
export const DOTENV = '.env';
export const MASK = '••••••••••••••••••••';
export const COPIED = 'Copied. Clipboard clears in 30 seconds.';

/** The value a row shows: the first stored secret, else the first known field. */
export const primaryField = (c: CredentialView) =>
  c.fields.find((f) => f.secret && f.available) ?? c.fields.find((f) => f.available);
export const shown = (f: KeyField, secret?: Record<string, string>) =>
  !f.available ? undefined : f.secret ? secret?.[f.key] : (f.value ?? undefined);

/** Reveals a key's secret fields until 30 seconds pass or the window loses focus. */
export function useReveal(id: string, perform: Controller['perform']) {
  const [secret, setSecret] = useState<Record<string, string>>();
  useEffect(() => {
    if (!secret) return;
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
  const toggle = () => {
    if (secret) setSecret(undefined);
    else
      void perform(() =>
        request<{ fields: Record<string, string> }>({ kind: 'reveal', credential_id: id }),
      ).then((s) => {
        if (s) setSecret(s.fields);
      });
  };
  return [secret, toggle] as const;
}
