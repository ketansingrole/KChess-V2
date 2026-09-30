import { defineConfig } from 'electron-vite'

export default defineConfig({
  main: {},
  preload: { build: { rollupOptions: { output: { format: 'cjs' } } } },
  renderer: {
    // electron-vite still needs an HTML entry; the actual renderer runs through Nuxt.
    server: { open: false },
    preview: { open: false },
  },
})
