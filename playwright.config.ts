import { defineConfig } from '@playwright/test';

const PORT = 4173;
const ORIGIN = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  expect: {
    timeout: 7_500,
  },
  reporter: [['list']],
  outputDir: 'test-results',
  use: {
    baseURL: ORIGIN,
    browserName: 'chromium',
    headless: true,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'off',
  },
  webServer: {
    command: 'npm run build:web && npx vite preview --host 127.0.0.1 --port 4173 --strictPort',
    url: ORIGIN,
    reuseExistingServer: false,
    timeout: 60_000,
  },
});
