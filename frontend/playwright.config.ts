import { defineConfig, devices } from '@playwright/test';

/**
 * End-to-end journeys against the built release binary.
 *
 * `webServer` spawns the actual Rust/Axum binary with its embedded frontend
 * against a scratch SQLite database under test-results/scratch — the same
 * layout a self-hosted instance gets, and a fresh one per run. Readiness is
 * the server's own `/health/ready` (it names the backend it reached).
 *
 * Failures retain traces and screenshots (see `use` below), so a red run
 * explains itself after the fact.
 */
export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [['list']],
  use: {
    baseURL: 'http://127.0.0.1:8173',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: {
    command: 'bash e2e/serve-scratch.sh',
    url: 'http://127.0.0.1:8173/health/ready',
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
