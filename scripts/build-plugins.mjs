import { run } from './run.mjs';
import { compilePlugins } from './compile-plugins.mjs';
compilePlugins();
run('cargo', ['run', '-p', 'xtask', '--', 'package']);
