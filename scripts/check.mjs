import { run } from './run.mjs';
run('cargo', ['fmt', '--all', '--check']);
run('npm', ['run', 'format:check']);
run('node', ['scripts/verify-plugins.mjs']);
run('cargo', ['run', '-p', 'xtask', '--', 'verify']);
run('cargo', ['run', '-p', 'xtask', '--', 'types']);
run('npm', ['run', 'build']);
run('cargo', ['clippy', '--workspace', '--all-targets', '--', '-D', 'warnings']);
if (process.platform === 'darwin') {
  // The CLI journeys open each vault from two programs, the test and getkeys. macOS asks the user
  // before another unsigned program reads a keychain item, which blocks unattended runs.
  run('cargo', ['test', '--workspace', '--exclude', 'getkeys']);
  run('cargo', ['test', '-p', 'getkeys', '--bins']);
} else {
  run('cargo', ['test', '--workspace']);
}
run('npm', ['test']);
run('cargo', ['build', '-p', 'getkeys-desktop', '-p', 'getkeys']);
run('npm', ['run', 'test:e2e']);
run('npm', ['audit', '--audit-level=high']);
