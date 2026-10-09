import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'
import { coreAliases } from './tooling/core-aliases.ts'

export default defineConfig({
  plugins: [vue()],
  resolve: { alias: coreAliases },
  test: {
    include: [
      'tests/unit/**/*.test.{ts,mjs}',
      'core/tests/unit/**/*.test.{ts,mjs}',
      'hosts/node/tests/unit/**/*.test.{ts,mjs}',
      'apps/*/tests/unit/**/*.test.{ts,mjs}',
    ],
    environment: 'happy-dom',
    setupFiles: ['apps/desktop/tests/unit/setup.ts'],
    reporters: process.env.CI ? ['default', 'junit'] : ['default'],
    outputFile: { junit: 'test-results/unit.xml' },
    restoreMocks: true,
    clearMocks: true,
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json-summary'],
      reportsDirectory: 'test-results/coverage',
      include: [
        'apps/desktop/app/**/*.ts',
        'apps/desktop/app/**/*.vue',
        'core/src/services/**/*.ts',
        'apps/desktop/electron/main/**/*.ts',
        'core/src/domain/**/*.ts',
        'core/src/contracts/**/*.ts',
        'hosts/node/src/**/*.ts',
        'apps/cli/src/**/*.ts',
        'apps/desktop/contracts/**/*.ts',
      ],
      exclude: [
        'core/src/services/appIconSvg.ts',
        'apps/desktop/electron/renderer/**',
        '**/*.d.ts',
        '**/tests/**',
      ],
      thresholds: {
        // Baseline from 2026-10-06 (48% lines). Ratchet upward as suites grow;
        // these fail the run on real regressions, not on noise.
        lines: 45,
        functions: 32,
        branches: 30,
        statements: 43,
      },
    },
  },
})
