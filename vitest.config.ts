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
      // Floors are whole percentages rounded down from the measured totals of `pnpm run test:unit`
      // (2026-10-10, after the storage tests and the IPC checks moved to Rust: lines 41.36%,
      // functions 32.47%, branches 24.98%, statements 40.13%; the measure moves by about a point
      // between runs). They gate the TypeScript that remains (apps,
      // bindings, hosts); the Rust workspace has its own gate, `pnpm run check:rust-coverage`.
      // Ratchet them up as the measured totals rise.
      thresholds: {
        lines: 41,
        functions: 32,
        branches: 24,
        statements: 40,
      },
    },
  },
})
