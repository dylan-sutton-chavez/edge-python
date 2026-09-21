import { defineConfig, devices } from '@playwright/test'

// Its own port, so a run never borrows or disturbs the dev server on 4322.
const PORT = 4323
const BASE = `https://127.0.0.1:${PORT}`

// Specs with no renderer, run once beside the http suite instead of three times.
const NODE_ONLY = /(http|convention)\.spec\.ts/

// One D1 behind every test, so they share the database and run one at a time.
export default defineConfig({
  testDir: './tests',
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: process.env.CI ? 'github' : 'list',
  // Sessions ride on __Host- cookies, which are only stored over https, self-signed included.
  use: { baseURL: BASE, ignoreHTTPSErrors: true },
  projects: [
    // No renderer here, so it runs once, and it sends the origin a browser would have sent.
    { name: 'http', testMatch: NODE_ONLY, use: { extraHTTPHeaders: { origin: BASE } } },
    { name: 'chromium', testIgnore: NODE_ONLY, use: { ...devices['Desktop Chrome'] } },
    { name: 'firefox', testIgnore: NODE_ONLY, use: { ...devices['Desktop Firefox'] } },
    { name: 'webkit', testIgnore: NODE_ONLY, use: { ...devices['Desktop Safari'] } }
  ],
  webServer: {
    command: `npm run build && npx wrangler dev --local-protocol https --ip 127.0.0.1 --port ${PORT}`,
    url: `${BASE}/api/health`,
    ignoreHTTPSErrors: true,
    reuseExistingServer: false,
    timeout: 300000
  }
})
