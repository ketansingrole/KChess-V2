import { defineConfig } from 'vite'

export default defineConfig({
  build: {
    ssr: true,
    target: 'node24',
    outDir: 'core/dist',
    emptyOutDir: true,
    rollupOptions: {
      input: {
        index: 'core/src/index.ts',
        logger: 'core/src/services/logger.ts',
        puzzleWorker: 'core/src/services/puzzleWorker.ts',
        voiceModelWorker: 'core/src/services/voiceModelWorker.ts',
        gameSession: 'core/src/domain/gameSession.ts',
        gameArchive: 'core/src/domain/gameArchive.ts',
        onlineGame: 'core/src/domain/onlineGame.ts',
        puzzleSession: 'core/src/domain/puzzleSession.ts',
      },
      output: { format: 'es', entryFileNames: '[name].js', chunkFileNames: '[name]-[hash].js' },
    },
  },
})
