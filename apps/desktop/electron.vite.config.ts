import { defineConfig } from 'electron-vite'
import { fileURLToPath } from 'node:url'
import { packageAliases } from '../../tooling/package-aliases.ts'

const entry = (path: string) => fileURLToPath(new URL(path, import.meta.url))
// The workspace packages compile into the bundles from source; only third-party deps stay external.
const workspacePackages = ['@kchess/native', '@kchess/rules', '@kchess/contracts']

export default defineConfig({
  main: {
    resolve: { alias: packageAliases },
    build: {
      externalizeDeps: { exclude: workspacePackages },
      rollupOptions: {
        input: {
          index: entry('electron/main/index.ts'),
        },
      },
    },
  },
  preload: {
    resolve: { alias: packageAliases },
    build: {
      externalizeDeps: { exclude: workspacePackages },
      rollupOptions: { input: entry('electron/preload/index.ts'), output: { format: 'cjs' } },
    },
  },
  renderer: {
    resolve: { alias: packageAliases },
    root: entry('electron/renderer'),
    build: { rollupOptions: { input: entry('electron/renderer/index.html') } },
    server: { open: false },
    preview: { open: false },
  },
})
