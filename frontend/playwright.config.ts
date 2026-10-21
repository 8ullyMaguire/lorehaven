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
      // The no-JavaScript journeys run in their own project, and this one
      // does not run them.
      testIgnore: /reader-no-js\.spec\.ts/,
    },
    {
      /**
       * The same browser with **JavaScript switched off**.
       *
       * The reading surface is server-rendered HTML (spec §8.5, §21.1):
       * *"the reader works without JavaScript"*. A single-page application
       * cannot be that surface — its content is fetched after the document
       * loads — so the reader is written by the server and this project is
       * what proves it.
       */
      name: 'no-js',
      use: {
        ...devices['Desktop Chrome'],
        javaScriptEnabled: false,
      },
      // **Only** the no-JavaScript journeys. The project's name is not
      // enough to exclude a suite from another project, so the selection is
      // by file -- which is also why the suite carries no conditional skip:
      // a suite that skips itself has to know the project's name, and the
      // file is the thing that knows.
      testMatch: /reader-no-js\.spec\.ts/,
    },
  ],
  webServer: {
    command: 'bash e2e/serve-scratch.sh',
    url: 'http://127.0.0.1:8173/health/ready',
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
