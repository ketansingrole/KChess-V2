import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: './tests/e2e',
  outputDir: process.env.KCHESS_PACKAGED_EXEC_PATH
    ? 'test-results/packaged-artifacts'
    : 'test-results/desktop-artifacts',
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: process.env.CI
    ? [
        ['github'],
        ['html', { open: 'never' }],
        [
          'json',
          {
            outputFile: process.env.KCHESS_PACKAGED_EXEC_PATH
              ? 'test-results/desktop-packaged.json'
              : 'test-results/desktop.json',
          },
        ],
      ]
    : 'list',
  use: { trace: 'retain-on-failure', screenshot: 'only-on-failure' },
})
