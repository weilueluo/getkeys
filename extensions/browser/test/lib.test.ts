import { describe, expect, it } from 'vitest';
import {
  Deduper,
  classifyNativeError,
  classifySelection,
  compilePatterns,
  describeHostState,
  extractCandidates,
  findSecrets,
  hashValue,
  isProminent,
  looksLikeSecret,
  maskValue,
  parsePatternsResponse,
  parsePingResponse,
  parseSaveResponse,
  suggestName,
  validateSaveRequest,
} from '../src/lib';

const OPENAI = 'sk-proj-Ab3dEf6hIj9kLm2nOp5qRs8tUv1wXy4z';
const ANTHROPIC = 'sk-ant-api03-Zx9Yw8Vu7Ts6Rq5Po4Nm3Lk2Ji1Hg0Fe-AbCdEf12';
const patterns = compilePatterns([
  { provider: 'official.anthropic', name: 'Anthropic', pattern: '^sk-ant-(api|admin)\\d\\d-' },
  {
    provider: 'official.openai',
    name: 'OpenAI',
    pattern: '^sk-(proj-|svcacct-|admin-)?[A-Za-z0-9_-]{20,}$',
  },
  { provider: 'official.github', name: 'GitHub', pattern: 'gh[pousr]_[A-Za-z0-9]{36}' },
]);

describe('compilePatterns', () => {
  it('drops invalid, unsafe and malformed entries', () => {
    const compiled = compilePatterns([
      { provider: 'ok', name: 'Ok', pattern: '^ok_[a-z0-9]{20}' },
      { provider: 'bad', name: 'Bad', pattern: '([' },
      { provider: 'any', name: 'Any', pattern: '.*' },
      { provider: 'loose', name: 'Loose', pattern: '[a-z]' },
      { provider: 'bad id!', name: 'X', pattern: '^x' },
      { provider: 'noname', pattern: '^y' },
      'nope',
    ]);
    expect(compiled.map((p) => p.provider)).toEqual(['ok']);
    expect(compilePatterns(null)).toEqual([]);
  });
});

describe('looksLikeSecret', () => {
  it('accepts realistic keys and rejects placeholders', () => {
    expect(looksLikeSecret(OPENAI)).toBe(true);
    expect(looksLikeSecret('sk-proj-xxxxxxxxxxxxxxxxxxxxxxxx1')).toBe(false);
    expect(looksLikeSecret('sk-proj-abcdefghijklmnopqrstu')).toBe(false);
    expect(looksLikeSecret('short1')).toBe(false);
    expect(looksLikeSecret('https://platform.openai.com/api-keys1')).toBe(false);
  });
});

describe('extractCandidates', () => {
  it('splits env assignments and trims punctuation', () => {
    expect(extractCandidates(`OPENAI_API_KEY=${OPENAI}.`)).toEqual([OPENAI]);
    expect(extractCandidates(`"${ANTHROPIC}", then`)).toEqual([ANTHROPIC]);
  });
});

describe('findSecrets', () => {
  it('matches anchored prefix patterns and full patterns', () => {
    const found = findSecrets(`Your key: ${ANTHROPIC}\nexport OPENAI_API_KEY=${OPENAI}`, patterns);
    expect(found).toEqual([
      { provider: 'official.anthropic', providerName: 'Anthropic', value: ANTHROPIC },
      { provider: 'official.openai', providerName: 'OpenAI', value: OPENAI },
    ]);
  });

  it('uses the pattern match for unanchored patterns', () => {
    const token = 'ghp_' + 'A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8';
    expect(findSecrets(`token ${token} here`, patterns)[0]).toMatchObject({
      provider: 'official.github',
      value: token,
    });
  });

  it('ignores masked keys, docs placeholders and duplicates', () => {
    expect(findSecrets('sk-proj-****abcd and sk-ant-api03-...', patterns)).toEqual([]);
    expect(findSecrets(`${OPENAI} ${OPENAI}`, patterns)).toHaveLength(1);
    expect(findSecrets(OPENAI, [])).toEqual([]);
  });

  it('works with the anchored key_pattern regexes providers ship', () => {
    const shipped = compilePatterns([
      {
        provider: 'official.openai',
        name: 'OpenAI',
        pattern:
          '^sk-((proj|svcacct|admin)-[A-Za-z0-9_-]{40,}|[A-Za-z0-9]{20}T3BlbkFJ[A-Za-z0-9]{20})$',
      },
      {
        provider: 'official.anthropic',
        name: 'Anthropic',
        pattern: '^sk-ant-(api|admin)\\d\\d-[A-Za-z0-9_-]{20,}$',
      },
      { provider: 'official.aws', name: 'AWS', pattern: '^(AKIA|ASIA)[A-Z0-9]{16}$' },
      { provider: 'official.deepseek', name: 'DeepSeek', pattern: '^sk-[a-f0-9]{32}$' },
      {
        provider: 'official.stripe',
        name: 'Stripe',
        pattern: '^(sk|rk)_(live|test)_[A-Za-z0-9]{20,}$',
      },
    ]);
    expect(shipped).toHaveLength(5);
    const keys = {
      'official.openai': `sk-proj-${'Ab3dEf6hIj9kLm2nOp5qRs8tUv1wXy4z'.repeat(2)}`,
      'official.anthropic': ANTHROPIC,
      'official.aws': ['AKIA', 'Z7Q3W5E2R8T4Y6U1'].join(''),
      'official.deepseek': `sk-${'0a1b2c3d'.repeat(4)}`,
      'official.stripe': ['sk', 'live', '51Hx9Kq2Lm3Np4Rs5Tu6Vw7'].join('_'),
    };
    const text = Object.values(keys)
      .map((v, i) => `KEY_${i}="${v}"`)
      .join('\n');
    expect(
      Object.fromEntries(findSecrets(text, shipped).map((m) => [m.provider, m.value])),
    ).toEqual(keys);
  });
});

describe('classifySelection', () => {
  it('recognises known keys and falls back to unknown', () => {
    expect(classifySelection(`  ${OPENAI}\n`, patterns)?.provider).toBe('official.openai');
    expect(classifySelection(`key is ${OPENAI}`, patterns)?.value).toBe(OPENAI);
    expect(classifySelection('custom-secret-value', patterns)).toEqual({
      provider: null,
      providerName: 'Key',
      value: 'custom-secret-value',
    });
    expect(classifySelection('   ', patterns)).toBeNull();
  });
});

describe('dedupe', () => {
  it('hashes stably and reports each value once', () => {
    expect(hashValue(OPENAI)).toBe(hashValue(OPENAI));
    expect(hashValue(OPENAI)).not.toBe(hashValue(ANTHROPIC));
    expect(hashValue(OPENAI)).not.toContain('sk-');
    const deduper = new Deduper();
    expect(deduper.add(OPENAI)).toBe(hashValue(OPENAI));
    expect(deduper.add(OPENAI)).toBeNull();
    expect(deduper.has(OPENAI)).toBe(true);
    expect(deduper.size).toBe(1);
  });
});

describe('maskValue', () => {
  it('keeps only a short prefix and suffix', () => {
    expect(maskValue(OPENAI)).toBe(`sk-proj-…${OPENAI.slice(-4)}`);
    expect(maskValue(OPENAI)).not.toContain(OPENAI.slice(8, -4));
  });
});

describe('suggestName', () => {
  const now = new Date(2026, 9, 10);
  it('prefers provider + date', () => {
    expect(suggestName({ providerName: 'OpenAI', title: 'API keys', now })).toBe(
      'OpenAI 2026-10-10',
    );
  });
  it('falls back to the page title, then hostname', () => {
    expect(suggestName({ title: 'Tokens | Acme Console', now })).toBe('Tokens 2026-10-10');
    expect(suggestName({ providerName: 'Key', title: '', hostname: 'www.acme.dev', now })).toBe(
      'acme.dev 2026-10-10',
    );
    expect(suggestName({ now })).toBe('Key 2026-10-10');
  });
});

describe('isProminent', () => {
  const base = { inDialog: false, inCode: false, readonlyField: false, copied: false };
  it('only flags freshly shown contexts', () => {
    expect(isProminent(base)).toBe(false);
    expect(isProminent({ ...base, inDialog: true })).toBe(true);
    expect(isProminent({ ...base, inCode: true })).toBe(true);
    expect(isProminent({ ...base, readonlyField: true })).toBe(true);
    expect(isProminent({ ...base, copied: true })).toBe(true);
  });
});

describe('validateSaveRequest', () => {
  const request = {
    type: 'save',
    provider: 'official.openai',
    name: ' OpenAI 2026 ',
    value: OPENAI,
    url: 'https://platform.openai.com/api-keys',
  };
  it('accepts and normalises a valid request', () => {
    expect(validateSaveRequest(request)).toEqual({
      ok: true,
      value: { ...request, name: 'OpenAI 2026' },
    });
    expect(validateSaveRequest({ ...request, provider: null, url: '' }).ok).toBe(true);
  });
  it('rejects malformed requests', () => {
    expect(validateSaveRequest(null).ok).toBe(false);
    expect(validateSaveRequest({ ...request, type: 'ping' }).ok).toBe(false);
    expect(validateSaveRequest({ ...request, provider: '../x' }).ok).toBe(false);
    expect(validateSaveRequest({ ...request, name: '  ' })).toEqual({
      ok: false,
      error: 'Enter a name.',
    });
    expect(validateSaveRequest({ ...request, value: '' }).ok).toBe(false);
    expect(validateSaveRequest({ ...request, value: 'x'.repeat(20_000) }).ok).toBe(false);
    expect(validateSaveRequest({ ...request, url: 'javascript:alert(1)' }).ok).toBe(false);
  });
});

describe('host responses', () => {
  it('parses ping', () => {
    expect(parsePingResponse({ ok: true, version: '0.1.0' })).toEqual({ ok: true, value: '0.1.0' });
    expect(parsePingResponse({ ok: false, error: 'locked' })).toEqual({
      ok: false,
      error: 'locked',
    });
    expect(parsePingResponse(undefined).ok).toBe(false);
  });
  it('parses patterns and drops invalid ones', () => {
    const parsed = parsePatternsResponse({
      ok: true,
      patterns: [
        { provider: 'official.openai', name: 'OpenAI', pattern: '^sk-' },
        { provider: 'x', name: 'X', pattern: '(' },
      ],
    });
    expect(parsed).toEqual({
      ok: true,
      value: [{ provider: 'official.openai', name: 'OpenAI', pattern: '^sk-' }],
    });
    expect(parsePatternsResponse({ ok: true }).ok).toBe(false);
  });
  it('parses save', () => {
    expect(parseSaveResponse({ ok: true, id: 'k1' })).toEqual({ ok: true, value: 'k1' });
    expect(parseSaveResponse({ ok: false, error: 'Vault locked' })).toEqual({
      ok: false,
      error: 'Vault locked',
    });
    expect(parseSaveResponse({ ok: true }).ok).toBe(false);
  });
});

describe('native errors', () => {
  it('classifies chrome.runtime.lastError messages', () => {
    expect(classifyNativeError('Specified native messaging host not found.')).toBe('missing');
    expect(classifyNativeError('Access to the specified native messaging host is forbidden.')).toBe(
      'forbidden',
    );
    expect(classifyNativeError('Native host has exited.')).toBe('error');
    expect(describeHostState('missing')).toContain('getkeys native-host install');
  });
});
