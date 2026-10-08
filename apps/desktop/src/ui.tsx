import { useEffect, useId, useRef, type ReactNode } from 'react';
import { KeyRound, X } from 'lucide-react';

export function ProviderMark({ name, small = false }: { name: string; small?: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={`provider-mark ${small ? 'small' : ''} provider-${name.toLowerCase()}`}
    >
      {name === 'Vercel'
        ? '▲'
        : name === 'Supabase'
          ? 'ϟ'
          : name === 'AWS'
            ? 'aws'
            : name === 'GitHub'
              ? 'gh'
              : name.slice(0, 2).toLowerCase()}
    </span>
  );
}
export function Modal({
  title,
  children,
  onClose,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = ref.current!;
    dialog.showModal();
    return () => dialog.close();
  }, []);
  return (
    <dialog ref={ref} className="modal" aria-labelledby={titleId} onCancel={onClose}>
      <div className="modal-heading">
        <h2 id={titleId}>{title}</h2>
        <button type="button" className="icon-button" aria-label="Close dialog" onClick={onClose}>
          <X size={18} />
        </button>
      </div>
      {children}
    </dialog>
  );
}
export function Empty({
  title,
  children,
  action,
}: {
  title: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      <span className="empty-icon">
        <KeyRound size={23} strokeWidth={1.4} />
      </span>
      <h2>{title}</h2>
      <p>{children}</p>
      {action}
    </div>
  );
}
export function humanize(value: string) {
  return value.replaceAll('_', ' ').replaceAll('.', ' · ');
}
export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
    </label>
  );
}
