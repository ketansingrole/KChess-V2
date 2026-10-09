import { defineConfig } from 'vite'

export default defineConfig({
  build: {
    ssr: true,
    target: 'node24',
    outDir: 'hosts/node/dist',
    emptyOutDir: true,
    rollupOptions: {
      external: [/^@kchess\/(?:core|node)(?:\/|$)/],
      input: {
        node: 'hosts/node/src/index.ts',
        nodeWorker: 'hosts/node/src/worker.ts',
      },
      output: { format: 'es', entryFileNames: '[name].js', chunkFileNames: '[name]-[hash].js' },
    },
  },
})
