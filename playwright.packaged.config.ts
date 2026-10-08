import { defineConfig } from '@playwright/test';
import base from './playwright.config';

export default defineConfig({
  ...base,
  webServer: undefined,
  projects: [
    { name: 'packaged', testMatch: 'native.spec.ts', metadata: { packaged: true }, workers: 1 },
  ],
});
