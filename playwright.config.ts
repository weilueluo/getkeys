import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './tests',
  timeout: 45_000,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL: 'http://127.0.0.1:1420',
    viewport: { width: 1280, height: 880 },
    trace: 'retain-on-failure',
  },
  webServer: {
    command: 'npm run dev',
    url: 'http://127.0.0.1:1420',
    reuseExistingServer: !process.env.CI,
  },
  projects: [
    { name: 'browser', testIgnore: 'native.spec.ts', use: { browserName: 'chromium' } },
    // Native tests drive the app's WebView2 over CDP, which only Windows offers.
    ...(process.platform === 'win32'
      ? [{ name: 'native', testMatch: 'native.spec.ts', workers: 1 }]
      : []),
  ],
});
