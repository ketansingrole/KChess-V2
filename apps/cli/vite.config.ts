import { defineConfig } from 'vite'

export default defineConfig({
  build: {
    ssr: true,
    target: 'node24',
    outDir: 'apps/cli/dist',
    emptyOutDir: true,
    rollupOptions: {
      external: [/^@kchess\/(?:native|rules|contracts|node)(?:\/|$)/],
      input: {
        cli: 'apps/cli/src/index.ts',
      },
      output: {
        banner: '#!/usr/bin/env node',
        format: 'es',
        entryFileNames: '[name].js',
        chunkFileNames: '[name]-[hash].js',
      },
    },
  },
})
