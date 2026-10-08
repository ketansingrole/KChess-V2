import { defineConfig } from 'vite'

/**
 * The headless core as a plain Node library (`out/core`), for frontends other than the desktop
 * app. Dependencies stay external; the puzzle worker is built beside the entry.
 */
export default defineConfig({
  build: {
    ssr: true,
    target: 'node24',
    outDir: 'out/core',
    emptyOutDir: true,
    rollupOptions: {
      input: { index: 'src/core/index.ts', puzzleWorker: 'src/core/puzzleWorker.ts' },
      output: {
        format: 'es',
        entryFileNames: '[name].js',
        chunkFileNames: 'chunks/[name]-[hash].js',
      },
    },
  },
})
