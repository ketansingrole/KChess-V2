import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  test: {
    include: ['tests/unit/**/*.test.{ts,mjs}'],
    environment: 'happy-dom',
    setupFiles: ['tests/unit/setup.ts'],
    reporters: process.env.CI ? ['default', 'junit'] : ['default'],
    outputFile: { junit: 'test-results/unit.xml' },
    restoreMocks: true,
    clearMocks: true,
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json-summary'],
      reportsDirectory: 'test-results/coverage',
      include: [
        'app/**/*.ts',
        'app/**/*.vue',
        'src/core/**/*.ts',
        'src/main/**/*.ts',
        'src/shared/**/*.ts',
      ],
      exclude: ['src/core/appIconSvg.ts', 'src/renderer/**', '**/*.d.ts', 'tests/**'],
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
