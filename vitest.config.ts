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
      // (2026-10-10, after the storage tests moved to Rust: lines 41.50%, functions 32.84%, branches
      // 25.06%, statements 40.34%; a run before the Rust-side rebuild measured 42.17% lines, so the
      // measure moves by about a point between runs). They gate the TypeScript that remains (apps,
      // bindings, hosts); the Rust workspace has its own gate, `pnpm run check:rust-coverage`.
      // Ratchet them up as the measured totals rise.
      thresholds: {
        lines: 41,
        functions: 32,
        branches: 25,
        statements: 40,
      },
    },
  },
})
