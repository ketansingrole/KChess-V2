import { defineConfig } from 'electron-vite'
import { fileURLToPath } from 'node:url'
import { coreAliases } from '../../tooling/core-aliases.ts'

const entry = (path: string) => fileURLToPath(new URL(path, import.meta.url))

export default defineConfig({
  main: {
    resolve: { alias: coreAliases },
    build: {
      // Compile the workspace core into main from source; only third-party deps stay external.
      externalizeDeps: { exclude: ['@kchess/core'] },
      rollupOptions: {
        input: {
          index: entry('electron/main/index.ts'),
          voiceModelWorker: entry('../../core/src/services/voiceModelWorker.ts'),
        },
      },
    },
  },
  preload: {
    resolve: { alias: coreAliases },
    build: {
      externalizeDeps: { exclude: ['@kchess/core'] },
      rollupOptions: { input: entry('electron/preload/index.ts'), output: { format: 'cjs' } },
    },
  },
  renderer: {
    resolve: { alias: coreAliases },
    root: entry('electron/renderer'),
    build: { rollupOptions: { input: entry('electron/renderer/index.html') } },
    server: { open: false },
    preview: { open: false },
  },
})
