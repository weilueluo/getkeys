import { describe, expect, it } from 'vitest';
import {
  buildEnvArgs,
  buildGetArgs,
  buildRunArgs,
  classifyCliError,
  cleanValue,
  cliErrorMessage,
  collectGroups,
  defaultField,
  describeRef,
  findRefAt,
  groupByProvider,
  lookupRef,
  parseKeyList,
  parseRef,
  refForField,
  shellWrap,
} from '../src/lib';

const LS = JSON.stringify([
  {
    id: 'k1',
    ref: 'getkeys://openai/default',
    name: 'default',
    provider: 'OpenAI',
    plugin_id: 'official.openai',
    account: 'acme',
    groups: ['Work'],
    fields: [{ key: 'token', label: 'API key', secret: true }],
    problem: null,
  },
  {
    id: 'k2',
    ref: 'getkeys://aws/deploy',
    name: 'deploy',
    provider: 'AWS',
    plugin_id: 'official.aws',
    account: null,
    groups: ['Work', 'CI'],
    fields: [
      { key: 'access_key_id', label: 'Access key ID', secret: false },
      { key: 'secret_access_key', label: 'Secret access key', secret: true },
    ],
    problem: 'Expired',
  },
  {
    id: 'k3',
    ref: 'getkeys://custom/My%20Token',
    name: 'My Token',
    provider: null,
    plugin_id: null,
    groups: [],
    fields: [],
    problem: null,
  },
  { id: 'bad', name: 'no ref' },
  'junk',
]);

describe('parseKeyList', () => {
  it('parses valid entries and skips malformed ones', () => {
    const keys = parseKeyList(LS);
    expect(keys.map((k) => k.id)).toEqual(['k1', 'k2', 'k3']);
    expect(keys[0]).toMatchObject({ pluginId: 'official.openai', account: 'acme', problem: null });
    expect(keys[1].fields[1]).toEqual({
      key: 'secret_access_key',
      label: 'Secret access key',
      secret: true,
    });
    expect(keys[2]).toMatchObject({ provider: 'Custom', account: null, fields: [] });
  });
  it('rejects non-JSON and non-array output', () => {
    expect(() => parseKeyList('oops')).toThrow(/invalid JSON/);
    expect(() => parseKeyList('{}')).toThrow(/unexpected/);
    expect(parseKeyList('[]')).toEqual([]);
  });
});

describe('refs', () => {
  it('parses provider, name and field', () => {
    expect(parseRef('getkeys://openai/default')).toEqual({
      provider: 'openai',
      name: 'default',
      field: null,
    });
    expect(parseRef('getkeys://aws/deploy/secret_access_key')).toEqual({
      provider: 'aws',
      name: 'deploy',
      field: 'secret_access_key',
    });
    expect(parseRef('getkeys://custom/My%20Token')?.name).toBe('My Token');
    expect(parseRef('getkeys://openai')).toBeNull();
    expect(parseRef('getkeys://a/b/c/d')).toBeNull();
    expect(parseRef('https://openai/default')).toBeNull();
  });
  it('builds field refs', () => {
    const [openai] = parseKeyList(LS);
    expect(refForField(openai, 'token')).toBe('getkeys://openai/default/token');
    expect(refForField(openai, null)).toBe('getkeys://openai/default');
  });
  it('finds a ref under the cursor', () => {
    const line = 'OPENAI_API_KEY="getkeys://openai/default/token" # see getkeys://aws/deploy.';
    expect(findRefAt(line, 20)).toEqual({
      ref: 'getkeys://openai/default/token',
      start: 16,
      end: 46,
    });
    expect(findRefAt(line, 2)).toBeNull();
    expect(findRefAt(line, line.length - 3)?.ref).toBe('getkeys://aws/deploy');
  });
  it('looks up refs against the key list', () => {
    const keys = parseKeyList(LS);
    expect(lookupRef(keys, 'getkeys://openai/default')?.key.id).toBe('k1');
    expect(lookupRef(keys, 'getkeys://OpenAI/default/token')?.field?.label).toBe('API key');
    expect(lookupRef(keys, 'getkeys://openai/default/nope')).toBeNull();
    expect(lookupRef(keys, 'getkeys://custom/My Token')?.key.id).toBe('k3');
    expect(lookupRef(keys, 'getkeys://openai/other')).toBeNull();
  });
});

describe('describeRef', () => {
  it('shows metadata, never values, and escapes markdown', () => {
    const keys = parseKeyList(LS);
    const text = describeRef(keys, 'getkeys://aws/deploy/secret_access_key');
    expect(text).toContain('**deploy** · AWS');
    expect(text).toContain('Field: Secret access key');
    expect(text).toContain('Groups: Work, CI');
    expect(text).toContain('$(warning) Expired');
    expect(describeRef(keys, 'getkeys://x/y')).toContain('unknown reference');
    keys[0].name = '[click](command:evil)';
    expect(describeRef(keys, 'getkeys://openai/default')).toContain(
      '\\[click\\]\\(command:evil\\)',
    );
  });
});

describe('grouping', () => {
  it('groups by provider with Custom last and counts problems', () => {
    const groups = groupByProvider(parseKeyList(LS));
    expect(groups.map((g) => [g.provider, g.keys.length, g.problems])).toEqual([
      ['AWS', 1, 1],
      ['OpenAI', 1, 0],
      ['Custom', 1, 0],
    ]);
  });
  it('collects distinct groups', () => {
    expect(collectGroups(parseKeyList(LS))).toEqual(['CI', 'Work']);
  });
  it('picks the secret field by default', () => {
    const keys = parseKeyList(LS);
    expect(defaultField(keys[1])?.key).toBe('secret_access_key');
    expect(defaultField(keys[2])).toBeNull();
  });
});

describe('CLI arguments', () => {
  const selection = { groups: ['Work'], keys: ['getkeys://openai/default'] };
  it('wraps commands in a platform shell', () => {
    expect(shellWrap('npm test', 'win32')).toEqual(['cmd.exe', '/d', '/s', '/c', 'npm test']);
    expect(shellWrap('npm test', 'linux')).toEqual(['/bin/sh', '-c', 'npm test']);
  });
  it('builds run, env and get invocations', () => {
    expect(buildRunArgs(selection, 'npm test', 'darwin')).toEqual([
      'run',
      '--group',
      'Work',
      '--key',
      'getkeys://openai/default',
      '--',
      '/bin/sh',
      '-c',
      'npm test',
    ]);
    expect(buildEnvArgs({ groups: ['A', 'B'], keys: [] }, 'C:\\p\\.env')).toEqual([
      'env',
      '--group',
      'A',
      '--group',
      'B',
      '-o',
      'C:\\p\\.env',
    ]);
    expect(buildGetArgs('getkeys://aws/deploy', 'secret_access_key')).toEqual([
      'get',
      'getkeys://aws/deploy',
      '--field',
      'secret_access_key',
      '--show',
    ]);
    expect(buildGetArgs('getkeys://openai/default', null)).toEqual([
      'get',
      'getkeys://openai/default',
      '--show',
    ]);
  });
  it('strips only the trailing newline from values', () => {
    expect(cleanValue('abc \r\n')).toBe('abc ');
    expect(cleanValue('abc')).toBe('abc');
  });
});

describe('CLI errors', () => {
  it('detects a missing binary', () => {
    expect(
      classifyCliError(Object.assign(new Error('spawn getkeys ENOENT'), { code: 'ENOENT' })),
    ).toBe('missing');
    expect(classifyCliError(Object.assign(new Error('failed'), { code: 1 }))).toBe('error');
  });
  it('prefers the first stderr line', () => {
    const error = Object.assign(new Error('Command failed: getkeys ls'), {
      code: 1,
      stderr: '\nerror: vault is locked\nhint: open the app\n',
    });
    expect(cliErrorMessage(error)).toBe('vault is locked');
    expect(cliErrorMessage(new Error('Command failed\nmore'))).toBe('Command failed');
    expect(cliErrorMessage({ code: 'ENOENT' })).toBe('GetKeys CLI not found.');
  });
});
