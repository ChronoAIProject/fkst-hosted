// Playwright configuration for Evolution journeys.
//
// This is the section 12.1.2 "consumption" pattern: generated journeys live
// under `.fkst/evolution/journeys/`, a dot-directory conventional tooling does
// not discover by name, and the consumer — here, a test runner — is pointed at
// that root through its OWN configuration. Evolution never copies generated
// files out to conventional locations, because a copy is a second maintained
// artifact that immediately starts to rot.
//
// Every capture-affecting setting below is PINNED rather than defaulted. Section
// 23.6 requires each capture to record its viewport, device scale factor, locale
// and theme; a default that changed with a Playwright upgrade would silently
// alter every screenshot and every video frame while the fingerprints reported
// the artifacts as current.

import { defineConfig, devices } from '@playwright/test';

const port = Number(process.env.FKST_EVOLUTION_JOURNEY_PORT ?? '4173');
// `localhost`, not `127.0.0.1`: vite preview binds the hostname, and on a
// dual-stack machine it answers on ::1 first. Probing the IPv4 literal times out
// against a server that is in fact up — which presents as a 4-minute webServer
// timeout with a successful build in the log.
const baseURL = `http://localhost:${port}`;

export default defineConfig({
  // Relative to this file: <repo>/.fkst/evolution/journeys
  testDir: '../../.fkst/evolution/journeys',
  // Raw run artifacts (videos, traces) are BUILD output, not Evolution state.
  // Section 12.2 forbids temporary media frames under the Evolution root.
  outputDir: './out/journeys',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  // No retries: a journey is evidence. A result that only holds on the second
  // attempt is a flaky demo, and section 23.5 asks for determinism instead.
  retries: 0,
  workers: 1,
  reporter: 'line',
  use: {
    baseURL,
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
    locale: 'en-US',
    timezoneId: 'UTC',
    colorScheme: 'dark',
    trace: 'off',
    video: { mode: 'on', size: { width: 1440, height: 900 } },
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 },
    },
  ],
  webServer: {
    // The built SPA, not the dev server: a demo should show the artifact users
    // receive. `cwd` is relative to this config file.
    command: `npm run build && npm run preview -- --port ${port} --strictPort`,
    cwd: '../../frontend',
    url: baseURL,
    reuseExistingServer: !process.env.CI,
    timeout: 240_000,
  },
});
