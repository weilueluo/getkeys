import {
  createContext,
  forwardRef,
  useContext,
  useEffect,
  useId,
  useRef,
  type ReactNode,
} from 'react';
import { Check, KeyRound, Minus, Search, TriangleAlert, X } from 'lucide-react';
import type { ConfigField } from './types';

// Provider logo paths from Simple Icons (CC0). AWS has no Simple Icons entry, so it uses a wordmark.
const logos: Record<string, string> = {
  github:
    'M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12',
  vercel: 'm12 1.608 12 20.784H0Z',
  supabase:
    'M11.9 1.036c-.015-.986-1.26-1.41-1.874-.637L.764 12.05C-.33 13.427.65 15.455 2.409 15.455h9.579l.113 7.51c.014.985 1.259 1.408 1.873.636l9.262-11.653c1.093-1.375.113-3.403-1.645-3.403h-9.642z',
};

export function ProviderMark({ name, small = false }: { name: string; small?: boolean }) {
  const slug = name.toLowerCase();
  const path = logos[slug];
  return (
    <span aria-hidden="true" className={`provider-mark ${small ? 'small' : ''} provider-${slug}`}>
      {path ? (
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d={path} />
        </svg>
      ) : slug === 'custom' ? (
        <KeyRound size={small ? 14 : 18} strokeWidth={1.75} />
      ) : (
        <span className="provider-letters">{slug === 'aws' ? 'aws' : name.slice(0, 2)}</span>
      )}
    </span>
  );
}

export function KeyGlyph({ size = 18 }: { size?: number }) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
    >
      <circle cx="8" cy="12" r="4.5" />
      <path d="M12.5 12H21M18 12v3.5" />
      <circle cx="8" cy="12" r="1.6" stroke="none" style={{ fill: 'var(--signal)' }} />
    </svg>
  );
}

export function PageHeader({
  title,
  description,
  children,
}: {
  title: string;
  description?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <header className="page-header">
      <div>
        <h1>{title}</h1>
        {description && <p>{description}</p>}
      </div>
      {children && <div className="page-header-actions">{children}</div>}
    </header>
  );
}

const shortcut =
  typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.userAgent) ? '⌘K' : 'Ctrl K';
export const SearchField = forwardRef<
  HTMLInputElement,
  { value: string; onChange: (value: string) => void; placeholder: string }
>(function SearchField({ value, onChange, placeholder }, ref) {
  return (
    <div className="search">
      <Search size={15} strokeWidth={1.75} />
      <input
        ref={ref}
        aria-label="Search"
        placeholder={placeholder}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        spellCheck={false}
      />
      {value ? (
        <button
          type="button"
          className="search-clear"
          aria-label="Clear search"
          onClick={() => onChange('')}
        >
          <X size={13} />
        </button>
      ) : (
        <kbd>{shortcut}</kbd>
      )}
    </div>
  );
});

/** Lets dialogs show the app's error, since everything behind an open dialog is inert. */
export const DialogHost = createContext<{ error: string; opened: (delta: number) => void }>({
  error: '',
  opened: () => {},
});

export function Modal({
  title,
  children,
  onClose,
  description,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  description?: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const { error, opened } = useContext(DialogHost);
  useEffect(() => {
    const dialog = ref.current!;
    dialog.showModal();
    opened(1);
    return () => {
      dialog.close();
      opened(-1);
    };
  }, [opened]);
  return (
    <dialog
      ref={ref}
      className="modal"
      aria-labelledby={titleId}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      <div className="modal-heading">
        <div>
          <h2 id={titleId}>{title}</h2>
          {description && <p className="modal-description">{description}</p>}
        </div>
        <button type="button" className="icon-button" aria-label="Close dialog" onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      {error && (
        <div role="alert" className="alert">
          <TriangleAlert size={16} strokeWidth={1.75} />
          <span>{error}</span>
        </div>
      )}
      {children}
    </dialog>
  );
}

export function Tabs<T extends string>({
  label,
  value,
  onChange,
  tabs,
}: {
  label: string;
  value: T;
  onChange: (value: T) => void;
  tabs: { id: T; label: string; count?: number }[];
}) {
  return (
    <div className="tabs" role="tablist" aria-label={label}>
      {tabs.map((t) => (
        <button
          key={t.id}
          type="button"
          role="tab"
          aria-selected={value === t.id}
          className="tab"
          onClick={() => onChange(t.id)}
        >
          {t.label}
          {t.count !== undefined && <span className="count">{t.count}</span>}
        </button>
      ))}
    </div>
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
      <h2>{title}</h2>
      <p>{children}</p>
      {action}
    </div>
  );
}

export function humanize(value: string) {
  return value.replaceAll('_', ' ').replaceAll('.', ' · ');
}

export function sentence(value: string) {
  // Older activity entries were logged as `plugin.*`.
  const text = humanize(value.replace(/^plugin\./, 'provider.'));
  return text.charAt(0).toUpperCase() + text.slice(1);
}

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
    </label>
  );
}

/** The fields a person types or pastes; the rest are derived by the provider. */
export const inputs = (fields: ConfigField[]) => fields.filter((f) => f.required || f.secret);

/** A checkbox styled like the rest of the app; `mixed` shows a partial selection. */
export function Checkbox({
  checked,
  mixed = false,
  onChange,
  label,
}: {
  checked: boolean;
  mixed?: boolean;
  onChange: () => void;
  label: ReactNode;
}) {
  return (
    <label className="checkbox">
      <input
        type="checkbox"
        checked={checked}
        ref={(el) => {
          if (el) el.indeterminate = mixed;
        }}
        onChange={onChange}
      />
      <span className="checkbox-box" aria-hidden="true">
        {mixed ? <Minus size={12} strokeWidth={3} /> : <Check size={12} strokeWidth={3} />}
      </span>
      {label}
    </label>
  );
}
