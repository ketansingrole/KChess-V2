import { defineConfig } from 'electron-vite'
import { fileURLToPath } from 'node:url'

const entry = (path: string) => fileURLToPath(new URL(path, import.meta.url))

export default defineConfig({
  main: {
    build: {
      rollupOptions: {
        input: {
          index: entry('electron/main/index.ts'),
          puzzleWorker: entry('../../core/src/services/puzzleWorker.ts'),
          voiceModelWorker: entry('electron/main/voiceModelWorker.ts'),
        },
      },
    },
  },
  preload: {
    build: {
      rollupOptions: { input: entry('electron/preload/index.ts'), output: { format: 'cjs' } },
    },
  },
  renderer: {
    root: entry('electron/renderer'),
    build: { rollupOptions: { input: entry('electron/renderer/index.html') } },
    server: { open: false },
    preview: { open: false },
  },
})
