import { resolve } from 'node:path'
import { defineConfig } from 'electron-vite'
import vue from '@vitejs/plugin-vue'
import ui from '@nuxt/ui/vite'

export default defineConfig({
  main: {},
  preload: { build: { rollupOptions: { output: { format: 'cjs' } } } },
  renderer: {
    resolve: { alias: { '@': resolve('src/renderer/src') } },
    plugins: [vue(), ui({ icon: { clientBundle: { scan: true } } })]
  }
})
