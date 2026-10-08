import { run } from './run.mjs';
run('npm', ['run', 'tauri', '--workspace', '@getkeys/desktop', '--', ...process.argv.slice(2)]);
