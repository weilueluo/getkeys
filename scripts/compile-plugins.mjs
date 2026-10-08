import { readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { resolve } from 'node:path';
import { run } from './run.mjs';

export function compilePlugins() {
  const names = readdirSync('plugins', { withFileTypes: true })
    .filter((p) => p.isDirectory())
    .map((p) => p.name)
    .sort();
  const cargoHome = resolve(process.env.CARGO_HOME ?? `${homedir()}/.cargo`);
  run(
    'cargo',
    [
      'build',
      '--locked',
      '--release',
      '--target',
      'wasm32-unknown-unknown',
      ...names.flatMap((p) => ['-p', `getkeys-plugin-${p}`]),
    ],
    {
      env: {
        CARGO_ENCODED_RUSTFLAGS: [
          `--remap-path-prefix=${resolve('.')}=/getkeys`,
          `--remap-path-prefix=${cargoHome}=/cargo`,
        ].join('\x1f'),
      },
    },
  );
  return names;
}
