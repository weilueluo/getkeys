// Pure logic for the VS Code extension (no `vscode` import) so it can be unit tested.

export interface KeyField {
  key: string;
  label: string;
  secret: boolean;
}

export interface KeyInfo {
  id: string;
  ref: string;
  name: string;
  provider: string;
  pluginId: string | null;
  account: string | null;
  groups: string[];
  fields: KeyField[];
  problem: string | null;
}

export interface ParsedRef {
  provider: string;
  name: string;
  field: string | null;
}

export interface ProviderGroup {
  provider: string;
  keys: KeyInfo[];
  problems: number;
}

export interface Selection {
  groups: string[];
  keys: string[];
}

export const REF_SCHEME = 'getkeys://';
export const CUSTOM_PROVIDER = 'Custom';
const REF_TEXT = /getkeys:\/\/[^\s"'`<>()[\]{},;]+/g;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function str(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null;
}

function decode(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}

/** Parses `getkeys://<provider>/<key-name>[/<field>]`. */
export function parseRef(text: string): ParsedRef | null {
  if (!text.startsWith(REF_SCHEME)) return null;
  const parts = text.slice(REF_SCHEME.length).replace(/\/+$/, '').split('/');
  if (parts.length < 2 || parts.length > 3 || parts.some((part) => !part)) return null;
  const [provider, name, field] = parts.map(decode);
  return { provider, name, field: field ?? null };
}

/** Appends a field to a key ref (`getkeys://openai/default` + `token`). */
export function refForField(key: KeyInfo, field: string | null): string {
  const base = key.ref.replace(/\/+$/, '');
  return field ? `${base}/${encodeURIComponent(field)}` : base;
}

/** Finds a `getkeys://` ref spanning `character` on a line of text. */
export function findRefAt(
  line: string,
  character: number,
): { ref: string; start: number; end: number } | null {
  for (const match of line.matchAll(REF_TEXT)) {
    const start = match.index ?? 0;
    const ref = match[0].replace(/[.:]+$/, '');
    const end = start + ref.length;
    if (character >= start && character <= end) return { ref, start, end };
  }
  return null;
}

/** Validates `getkeys ls --json` output; malformed entries are skipped. */
export function parseKeyList(stdout: string): KeyInfo[] {
  let data: unknown;
  try {
    data = JSON.parse(stdout);
  } catch {
    throw new Error('GetKeys CLI returned invalid JSON.');
  }
  if (!Array.isArray(data)) throw new Error('GetKeys CLI returned an unexpected key list.');
  const keys: KeyInfo[] = [];
  for (const item of data) {
    if (!isRecord(item)) continue;
    const id = str(item.id);
    const ref = str(item.ref);
    const name = str(item.name);
    if (!id || !ref || !name || !ref.startsWith(REF_SCHEME)) continue;
    const fields = Array.isArray(item.fields)
      ? item.fields.filter(isRecord).flatMap((field) => {
          const key = str(field.key);
          if (!key) return [];
          return [{ key, label: str(field.label) ?? key, secret: field.secret === true }];
        })
      : [];
    keys.push({
      id,
      ref,
      name,
      provider: str(item.provider) ?? CUSTOM_PROVIDER,
      pluginId: str(item.plugin_id),
      account: str(item.account),
      groups: Array.isArray(item.groups)
        ? item.groups.filter((g): g is string => typeof g === 'string' && !!g.trim())
        : [],
      fields,
      problem: str(item.problem),
    });
  }
  return keys;
}

/** Groups keys by provider (alphabetical, Custom last); keys sorted by name. */
export function groupByProvider(keys: KeyInfo[]): ProviderGroup[] {
  const map = new Map<string, KeyInfo[]>();
  for (const key of keys) map.set(key.provider, [...(map.get(key.provider) ?? []), key]);
  return [...map.entries()]
    .map(([provider, list]) => ({
      provider,
      keys: [...list].sort((a, b) => a.name.localeCompare(b.name)),
      problems: list.filter((key) => key.problem).length,
    }))
    .sort((a, b) => {
      if (a.provider === CUSTOM_PROVIDER) return 1;
      if (b.provider === CUSTOM_PROVIDER) return -1;
      return a.provider.localeCompare(b.provider);
    });
}

export function collectGroups(keys: KeyInfo[]): string[] {
  return [...new Set(keys.flatMap((key) => key.groups))].sort((a, b) => a.localeCompare(b));
}

/** Resolves a ref to a listed key (and field, if the ref names one). */
export function lookupRef(
  keys: KeyInfo[],
  ref: string,
): { key: KeyInfo; field: KeyField | null } | null {
  const parsed = parseRef(ref);
  if (!parsed) return null;
  for (const key of keys) {
    const own = parseRef(key.ref);
    if (!own) continue;
    if (own.provider.toLowerCase() !== parsed.provider.toLowerCase() || own.name !== parsed.name) {
      continue;
    }
    if (!parsed.field) return { key, field: null };
    const field = key.fields.find((f) => f.key === parsed.field);
    return field ? { key, field } : null;
  }
  return null;
}

/** Hover text for a ref: metadata only, never the value. */
export function describeRef(keys: KeyInfo[], ref: string): string {
  const found = lookupRef(keys, ref);
  if (!found) return `**GetKeys** · unknown reference ${escapeMarkdown(ref)}`;
  const { key, field } = found;
  const lines = [`**${escapeMarkdown(key.name)}** · ${escapeMarkdown(key.provider)}`];
  if (field) lines.push(`Field: ${escapeMarkdown(field.label)}`);
  if (key.account) lines.push(`Account: ${escapeMarkdown(key.account)}`);
  if (key.groups.length) lines.push(`Groups: ${key.groups.map(escapeMarkdown).join(', ')}`);
  if (key.problem) lines.push(`$(warning) ${escapeMarkdown(key.problem)}`);
  return lines.join('\n\n');
}

export function escapeMarkdown(text: string): string {
  return text.replace(/[\\`*_{}[\]()#+\-.!|<>$]/g, '\\$&');
}

/** The field to copy by default: the first secret field, else the first field. */
export function defaultField(key: KeyInfo): KeyField | null {
  return key.fields.find((field) => field.secret) ?? key.fields[0] ?? null;
}

function selectionArgs(selection: Selection): string[] {
  return [
    ...selection.groups.flatMap((group) => ['--group', group]),
    ...selection.keys.flatMap((ref) => ['--key', ref]),
  ];
}

/** Wraps a shell command so `getkeys run` can start it on any platform. */
export function shellWrap(command: string, platform: NodeJS.Platform | string): string[] {
  return platform === 'win32' ? ['cmd.exe', '/d', '/s', '/c', command] : ['/bin/sh', '-c', command];
}

export function buildRunArgs(
  selection: Selection,
  command: string,
  platform: NodeJS.Platform | string,
): string[] {
  return ['run', ...selectionArgs(selection), '--', ...shellWrap(command, platform)];
}

export function buildEnvArgs(selection: Selection, output: string): string[] {
  return ['env', ...selectionArgs(selection), '-o', output];
}

export function buildGetArgs(ref: string, field: string | null): string[] {
  return ['get', ref, ...(field ? ['--field', field] : []), '--show'];
}

/** Strips the single trailing newline the CLI prints after a value. */
export function cleanValue(stdout: string): string {
  return stdout.replace(/\r?\n$/, '');
}

export type CliFailure = 'missing' | 'error';

export function classifyCliError(error: unknown): CliFailure {
  const code = isRecord(error) ? error.code : undefined;
  return code === 'ENOENT' || code === 'EACCES' ? 'missing' : 'error';
}

/** Short user-facing message for a failed CLI call (stderr first line, never stdout). */
export function cliErrorMessage(error: unknown): string {
  if (classifyCliError(error) === 'missing') return 'GetKeys CLI not found.';
  const stderr = isRecord(error) && typeof error.stderr === 'string' ? error.stderr.trim() : '';
  const first = stderr.split(/\r?\n/).find((line) => line.trim());
  if (first) return first.replace(/^error:\s*/i, '').slice(0, 300);
  return error instanceof Error ? error.message.split(/\r?\n/)[0] : 'GetKeys CLI failed.';
}
