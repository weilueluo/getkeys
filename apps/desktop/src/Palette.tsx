import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { Check, Search } from 'lucide-react';
import type { CredentialView, KeyField, Snapshot } from './types';
import { copyCredential, request } from './bridge';
import { hidePalette } from './shell';
import { COPIED, CUSTOM, listed, primaryField } from './secrets';
import { KeyGlyph, ProviderMark } from './ui';

type Entry = { c: CredentialView; field: KeyField; provider: string; place: string };

/** Scores `query` as an in-order subsequence of `text`, favouring runs and word starts; -1 when
 * it doesn't match. */
export function fuzzy(query: string, text: string) {
  const t = text.toLowerCase();
  let score = 0;
  let at = -1;
  for (const ch of query.toLowerCase()) {
    const next = t.indexOf(ch, at + 1);
    if (next < 0) return -1;
    score += next === at + 1 ? 3 : next === 0 || /[\s\-_./:·]/.test(t[next - 1]) ? 2 : 1;
    at = next;
  }
  return score;
}

/** Keys that have something to copy, with the provider and place they belong to. */
function entries(snapshot: Snapshot): Entry[] {
  const manifest = (id?: string) => snapshot.plugins.find((p) => p.manifest.id === id)?.manifest;
  return snapshot.credentials
    .flatMap((c) => {
      const field = primaryField(c);
      if (!field || !listed(c, snapshot.connections)) return [];
      if (c.credential_type === CUSTOM) return [{ c, field, provider: 'Custom', place: '' }];
      const connection = snapshot.connections.find((x) => x.id === c.connection_id);
      const provider = manifest(c.plugin_id || connection?.plugin_id)?.name;
      if (!provider) return [];
      return [
        { c, field, provider, place: c.resource_name || c.resource || connection?.name || '' },
      ];
    })
    .sort((a, b) => a.provider.localeCompare(b.provider) || a.c.name.localeCompare(b.c.name));
}

/** The Ctrl+Shift+K window: type to find a key, Enter copies it and the window hides. */
export function Palette() {
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [query, setQuery] = useState('');
  const [cursor, setCursor] = useState(0);
  const [copied, setCopied] = useState('');
  const [error, setError] = useState('');
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const listId = useId();
  const dismiss = useCallback(() => {
    clearTimeout(timer.current);
    setQuery('');
    setCursor(0);
    setCopied('');
    setError('');
    void hidePalette().catch(() => {});
  }, []);
  useEffect(() => {
    // The window is kept between uses, so each time it shows it reads the keys again.
    const load = () => {
      input.current?.focus();
      request<Snapshot>({ kind: 'snapshot' })
        .then(setSnapshot)
        .catch((e) => setError(String(e)));
    };
    load();
    window.addEventListener('focus', load);
    window.addEventListener('blur', dismiss);
    return () => {
      clearTimeout(timer.current);
      window.removeEventListener('focus', load);
      window.removeEventListener('blur', dismiss);
    };
  }, [dismiss]);
  const all = useMemo(() => (snapshot ? entries(snapshot) : []), [snapshot]);
  const results = useMemo(() => {
    const terms = query.trim().split(/\s+/).filter(Boolean);
    if (!terms.length) return all;
    return all
      .map((e) => {
        const text = `${e.c.name} ${e.provider} ${e.place}`;
        const scores = terms.map((t) => fuzzy(t, text));
        return { e, score: scores.includes(-1) ? -1 : scores.reduce((a, b) => a + b, 0) };
      })
      .filter((r) => r.score >= 0)
      .sort((a, b) => b.score - a.score)
      .map((r) => r.e);
  }, [all, query]);
  const active = Math.min(cursor, results.length - 1);
  useEffect(() => {
    list.current?.children[active]?.scrollIntoView?.({ block: 'nearest' });
  }, [active]);
  const copy = async (e: Entry) => {
    try {
      await copyCredential(e.c.id, e.field.key);
      setCopied(e.c.id);
      timer.current = setTimeout(dismiss, 700);
    } catch (err) {
      setError(String(err));
    }
  };
  const move = (delta: number) =>
    setCursor(Math.max(0, Math.min(results.length - 1, active + delta)));
  const optionId = (i: number) => `${listId}-${i}`;
  return (
    <main className="palette">
      <h1 className="visually-hidden">Quick copy</h1>
      <div className="palette-search">
        <Search size={16} strokeWidth={1.75} />
        <input
          ref={input}
          autoFocus
          role="combobox"
          aria-label="Search keys"
          aria-expanded="true"
          aria-controls={listId}
          aria-activedescendant={results.length ? optionId(active) : undefined}
          placeholder="Search keys"
          spellCheck={false}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setCursor(0);
          }}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
              e.preventDefault();
              move(e.key === 'ArrowDown' ? 1 : -1);
            } else if (e.key === 'Enter') {
              e.preventDefault();
              if (results[active]) void copy(results[active]);
            } else if (e.key === 'Escape') {
              e.preventDefault();
              dismiss();
            }
          }}
        />
        <kbd>esc</kbd>
      </div>
      <ul ref={list} id={listId} className="palette-list" role="listbox" aria-label="Keys">
        {results.map((e, i) => (
          <li
            key={e.c.id}
            id={optionId(i)}
            role="option"
            aria-selected={i === active}
            className="palette-item"
            onMouseMove={() => i !== active && setCursor(i)}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => void copy(e)}
          >
            <ProviderMark small name={e.provider} />
            <span className="palette-name">
              <strong>{e.c.name}</strong>
              <small>{e.place ? `${e.provider} · ${e.place}` : e.provider}</small>
            </span>
            {copied === e.c.id ? (
              <Check size={15} strokeWidth={2} className="palette-done" aria-label="Copied" />
            ) : (
              <span className="palette-field">{e.field.label}</span>
            )}
          </li>
        ))}
      </ul>
      {!results.length && (
        <div className="palette-empty">
          <KeyGlyph size={22} />
          <p>
            {!snapshot
              ? error || 'Opening your vault…'
              : query.trim()
                ? 'No matching keys'
                : 'No keys to copy yet'}
          </p>
        </div>
      )}
      <div className="palette-footer" role="status">
        {copied ? (
          <span className="palette-copied">
            <Check size={13} strokeWidth={2.25} /> {COPIED}
          </span>
        ) : error && snapshot ? (
          <span className="palette-error">{error}</span>
        ) : (
          <>
            <span>
              <kbd>↑</kbd>
              <kbd>↓</kbd> to move
            </span>
            <span>
              <kbd>↵</kbd> to copy
            </span>
          </>
        )}
      </div>
    </main>
  );
}
