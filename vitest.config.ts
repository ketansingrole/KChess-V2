import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'
import { packageAliases } from './tooling/package-aliases.ts'

export default defineConfig({
  plugins: [vue()],
  resolve: { alias: packageAliases },
  test: {
    include: [
      'tests/unit/**/*.test.{ts,mjs}',
      'tests/core/**/*.test.{ts,mjs}',
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
        'crates/kchess-node/js/**/*.ts',
        'apps/desktop/electron/main/**/*.ts',
        'crates/kchess-wasm/js/**/*.ts',
        'crates/kchess-contracts/ts/**/*.ts',
        'hosts/node/src/**/*.ts',
        'apps/cli/src/**/*.ts',
        'apps/desktop/contracts/**/*.ts',
      ],
      exclude: [
        'crates/kchess-node/js/appIconSvg.ts',
        'apps/desktop/electron/renderer/**',
        '**/*.d.ts',
        '**/tests/**',
      ],
      thresholds: {
        // Baseline from 2026-10-06 (48% lines). Ratchet upward as suites grow;
        // these fail the run on real regressions, not on noise.
        // 42, down from 45: the Lichess services moved to Rust (2026-10-10, 42.64% lines after the
        // move); the floor follows the TypeScript as it shrinks.
        // 41, down from 42: the core facade moved to Rust (2026-10-10, 41.77% lines after it).
        lines: 41,
        functions: 32,
        // 28, down from 29: the main-database storage moved to Rust (2026-10-10), taking well-covered
        // TypeScript branches with it (29.92% after the move, 31.92% before); its behaviour is
        // pinned by the store golden suites and Rust tests, which this TypeScript measure omits.
        // Engines also moved to Rust (2026-10-10, 28.78% after); the floor follows the TypeScript
        // as it shrinks, and a Rust coverage gate replaces it once the core's TypeScript is gone.
        // 25, down from 28: the Lichess services moved to Rust (2026-10-10, 25.73% branches after the
        // move); their behaviour is pinned by the Rust `lichess_*` integration tests.
        branches: 25,
        // 41, down from 43: the same move (2026-10-10, 41.57% statements after it).
        // 40, down from 41: the core facade moved to Rust (2026-10-10, 40.63% statements after it).
        statements: 40,
      },
    },
  },
})
