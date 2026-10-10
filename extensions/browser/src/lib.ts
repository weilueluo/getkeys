// Pure logic shared by the background worker, content script and popup. No chrome.* or DOM access here.

export const HOST_NAME = 'dev.getkeys.host';
export const MIN_SECRET_LENGTH = 20;
export const MAX_SECRET_LENGTH = 16_384;
export const INSTALL_HINT = 'Install the GetKeys app, then run `getkeys native-host install`.';

export interface ProviderPattern {
  provider: string;
  name: string;
  pattern: string;
}

export interface CompiledPattern {
  provider: string;
  name: string;
  re: RegExp;
}

export interface SecretMatch {
  provider: string | null;
  providerName: string;
  value: string;
}

export interface SaveRequest {
  type: 'save';
  provider: string | null;
  name: string;
  value: string;
  url: string;
}

export type HostState = 'connected' | 'missing' | 'forbidden' | 'error';

export type Result<T> = { ok: true; value: T } | { ok: false; error: string };

const PROVIDER_ID = /^[a-z0-9][a-z0-9._-]{0,99}$/i;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Compiles host-provided patterns, dropping invalid ones and ones that would match anything. */
export function compilePatterns(list: unknown): CompiledPattern[] {
  if (!Array.isArray(list)) return [];
  const compiled: CompiledPattern[] = [];
  for (const item of list) {
    if (!isRecord(item)) continue;
    const { provider, name, pattern } = item;
    if (typeof provider !== 'string' || !PROVIDER_ID.test(provider)) continue;
    if (typeof name !== 'string' || typeof pattern !== 'string' || !pattern) continue;
    let re: RegExp;
    try {
      re = new RegExp(pattern);
    } catch {
      continue;
    }
    if (re.test('') || re.test('a')) continue;
    compiled.push({ provider, name: name.trim() || provider, re });
  }
  return compiled;
}

const CANDIDATE = /[A-Za-z0-9_\-.+/]{16,}={0,2}/g;

/** Heuristic: long, mixed letters and digits, not a placeholder like `sk-xxxxxxxx…`. */
export function looksLikeSecret(token: string): boolean {
  if (token.length < MIN_SECRET_LENGTH || token.length > MAX_SECRET_LENGTH) return false;
  if (!/[A-Za-z]/.test(token) || !/\d/.test(token)) return false;
  if (/(.)\1{5,}/.test(token)) return false;
  if (/^(https?|www\.)/i.test(token) || token.includes('//')) return false;
  return new Set(token).size >= 10;
}

/** Splits text into tokens that could be keys (`OPENAI_API_KEY=sk-…` yields `sk-…`). */
export function extractCandidates(text: string): string[] {
  const out: string[] = [];
  for (const match of text.matchAll(CANDIDATE)) {
    const token = match[0].replace(/^[./]+|[./]+$/g, '');
    if (token.length >= MIN_SECRET_LENGTH) out.push(token);
  }
  return out;
}

/** Returns the first provider whose pattern accepts `candidate`, with the exact value to save. */
export function matchCandidate(candidate: string, patterns: CompiledPattern[]): SecretMatch | null {
  for (const { provider, name, re } of patterns) {
    const m = re.exec(candidate);
    if (!m) continue;
    // Full-value patterns give the key itself; prefix-only patterns (`^sk-ant-`) mean the whole token.
    const value = m[0].length >= MIN_SECRET_LENGTH ? m[0] : m.index === 0 ? candidate : null;
    if (value && looksLikeSecret(value)) return { provider, providerName: name, value };
  }
  return null;
}

/** Finds provider keys in free text. Each distinct value is reported once. */
export function findSecrets(text: string, patterns: CompiledPattern[]): SecretMatch[] {
  if (!patterns.length || text.length < MIN_SECRET_LENGTH) return [];
  const seen = new Set<string>();
  const found: SecretMatch[] = [];
  for (const candidate of extractCandidates(text)) {
    const match = matchCandidate(candidate, patterns);
    if (match && !seen.has(match.value)) {
      seen.add(match.value);
      found.push(match);
    }
  }
  return found;
}

/** Classifies a manually selected value: a known provider key, or an unknown secret-looking string. */
export function classifySelection(text: string, patterns: CompiledPattern[]): SecretMatch | null {
  const value = text.trim();
  if (!value || value.length > MAX_SECRET_LENGTH) return null;
  const match = /\s/.test(value) ? null : matchCandidate(value, patterns);
  if (match) return match;
  const found = findSecrets(value, patterns);
  if (found.length === 1) return found[0];
  return { provider: null, providerName: 'Key', value };
}

/** Stable, non-reversible-in-practice id for dedupe (cyrb53). Never used as a security boundary. */
export function hashValue(value: string): string {
  let h1 = 0xdeadbeef;
  let h2 = 0x41c6ce57;
  for (let i = 0; i < value.length; i++) {
    const ch = value.charCodeAt(i);
    h1 = Math.imul(h1 ^ ch, 2654435761);
    h2 = Math.imul(h2 ^ ch, 1597334677);
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
  return (4294967296 * (2097151 & h2) + (h1 >>> 0)).toString(36);
}

/** Tracks values already seen on a page. `add` returns the hash only the first time. */
export class Deduper {
  private readonly seen = new Set<string>();

  add(value: string): string | null {
    const hash = hashValue(value);
    if (this.seen.has(hash)) return null;
    this.seen.add(hash);
    return hash;
  }

  has(value: string): boolean {
    return this.seen.has(hashValue(value));
  }

  get size(): number {
    return this.seen.size;
  }
}

/** Short preview that is safe to show next to the key on the page (`sk-proj-…a1b2`). */
export function maskValue(value: string): string {
  const head = value.slice(0, Math.min(8, Math.floor(value.length / 4)));
  return `${head}…${value.slice(-4)}`;
}

function isoDate(now: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/** Suggests a key name: provider + date, or the page's own title / hostname for unknown keys. */
export function suggestName(input: {
  providerName?: string | null;
  title?: string;
  hostname?: string;
  now?: Date;
}): string {
  const date = isoDate(input.now ?? new Date());
  if (input.providerName && input.providerName !== 'Key') return `${input.providerName} ${date}`;
  const title = (input.title ?? '')
    .split(/\s+[|·–—-]\s+/)[0]
    .trim()
    .slice(0, 60);
  const base = title || (input.hostname ?? '').replace(/^www\./, '') || 'Key';
  return `${base} ${date}`;
}

export interface DetectionContext {
  inDialog: boolean;
  inCode: boolean;
  readonlyField: boolean;
  copied: boolean;
}

/** Keys shown in dialogs, code blocks, read-only fields or just copied look freshly created. */
export function isProminent(ctx: DetectionContext): boolean {
  return ctx.copied || ctx.inDialog || ctx.inCode || ctx.readonlyField;
}

/** Validates an outgoing save before it reaches the native host. */
export function validateSaveRequest(input: unknown): Result<SaveRequest> {
  if (!isRecord(input) || input.type !== 'save') return { ok: false, error: 'Invalid request.' };
  const { provider, name, value, url } = input;
  if (provider !== null && (typeof provider !== 'string' || !PROVIDER_ID.test(provider))) {
    return { ok: false, error: 'Invalid provider.' };
  }
  if (typeof name !== 'string' || !name.trim()) return { ok: false, error: 'Enter a name.' };
  if (name.trim().length > 200) return { ok: false, error: 'Name is too long.' };
  if (typeof value !== 'string' || !value.trim()) return { ok: false, error: 'Nothing to save.' };
  if (value.length > MAX_SECRET_LENGTH) return { ok: false, error: 'Value is too long.' };
  if (typeof url !== 'string' || url.length > 2048)
    return { ok: false, error: 'Invalid page URL.' };
  if (url && !/^https?:\/\//i.test(url)) return { ok: false, error: 'Invalid page URL.' };
  return {
    ok: true,
    value: { type: 'save', provider, name: name.trim(), value: value.trim(), url },
  };
}

export function parsePingResponse(response: unknown): Result<string> {
  if (isRecord(response) && response.ok === true) {
    return { ok: true, value: typeof response.version === 'string' ? response.version : '' };
  }
  return { ok: false, error: hostError(response) };
}

export function parsePatternsResponse(response: unknown): Result<ProviderPattern[]> {
  if (!isRecord(response) || response.ok !== true || !Array.isArray(response.patterns)) {
    return { ok: false, error: hostError(response) };
  }
  const patterns = compilePatterns(response.patterns).map(({ provider, name, re }) => ({
    provider,
    name,
    pattern: re.source,
  }));
  return { ok: true, value: patterns };
}

export function parseSaveResponse(response: unknown): Result<string> {
  if (isRecord(response) && response.ok === true && typeof response.id === 'string') {
    return { ok: true, value: response.id };
  }
  return { ok: false, error: hostError(response) };
}

function hostError(response: unknown): string {
  if (isRecord(response) && typeof response.error === 'string' && response.error.trim()) {
    return response.error.trim().slice(0, 300);
  }
  return 'GetKeys returned an unexpected response.';
}

/** Maps `chrome.runtime.lastError` messages from native messaging to a state. */
export function classifyNativeError(message: string): Exclude<HostState, 'connected'> {
  const text = message.toLowerCase();
  if (text.includes('not found')) return 'missing';
  if (text.includes('forbidden')) return 'forbidden';
  return 'error';
}

export function describeHostState(state: HostState, message = ''): string {
  switch (state) {
    case 'connected':
      return 'Connected';
    case 'missing':
      return INSTALL_HINT;
    case 'forbidden':
      return 'GetKeys does not allow this extension. Run `getkeys native-host install` again.';
    case 'error':
      return message || 'Could not reach GetKeys.';
  }
}
