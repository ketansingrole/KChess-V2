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
  },
})
