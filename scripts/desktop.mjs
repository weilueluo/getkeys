import { run } from './run.mjs';
const args = process.argv.slice(2);
if (args[0] === 'build') {
  // The installer ships the CLI as bin\getkeys.exe and offers to put it on PATH.
  run('cargo', ['build', '--release', '-p', 'getkeys']);
  args.push('--config', 'src-tauri/cli.conf.json');
}
run('npm', ['run', 'tauri', '--workspace', '@getkeys/desktop', '--', ...args]);
