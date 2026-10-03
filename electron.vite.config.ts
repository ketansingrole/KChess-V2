import { defineConfig } from 'electron-vite'

export default defineConfig({
  main: {
    build: {
      rollupOptions: {
        input: { index: 'src/main/index.ts', puzzleWorker: 'src/main/puzzleWorker.ts' },
      },
    },
  },
  preload: { build: { rollupOptions: { output: { format: 'cjs' } } } },
  renderer: {
    // electron-vite still needs an HTML entry; the actual renderer runs through Nuxt.
    server: { open: false },
    preview: { open: false },
  },
})
