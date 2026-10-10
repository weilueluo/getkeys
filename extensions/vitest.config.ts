import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['browser/test/**/*.test.ts', 'vscode/test/**/*.test.ts'],
    environment: 'node',
  },
});
