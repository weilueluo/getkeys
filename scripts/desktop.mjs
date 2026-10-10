import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { run } from './run.mjs';
const args = process.argv.slice(2);
if (args[0] === 'build') {
  // Every bundle ships the CLI as bin/getkeys(.exe) beside the provider registry, and the Windows
  // installer offers to put it on PATH.
  const at = args.findIndex((a) => a === '--target' || a === '-t' || a.startsWith('--target='));
  const target = at < 0 ? undefined : (args[at].split('=')[1] ?? args[at + 1]);
  const exe = process.platform === 'win32' ? 'getkeys.exe' : 'getkeys';
  // A debug app gets a debug CLI, which shares its build with the app's.
  const profile = args.includes('--debug') ? 'debug' : 'release';
  run('cargo', [
    'build',
    ...(profile === 'release' ? ['--release'] : []),
    '-p',
    'getkeys',
    ...(target ? ['--target', target] : []),
  ]);
  // Resource sources are relative to src-tauri.
  const cli = ['../../../target', target, profile, exe].filter(Boolean).join('/');
  const config = resolve('target/cli.conf.json');
  mkdirSync('target', { recursive: true });
  writeFileSync(config, JSON.stringify({ bundle: { resources: { [cli]: `bin/${exe}` } } }));
  args.push('--config', config);
}
run('npm', ['run', 'tauri', '--workspace', '@getkeys/desktop', '--', ...args]);
