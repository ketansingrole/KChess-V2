import { localIcons, voskWorkerPlugin } from './scripts/vosk-worker-plugin'

export default defineNuxtConfig({
  compatibilityDate: '2026-09-29',
  ssr: false,
  // Ignore model files left by older builds; voice models now live in the user's cache.
  nitro: { ignore: ['**/voice/model.tar.gz*'] },
  modules: ['@pinia/nuxt', '@nuxt/ui', '@nuxt/eslint'],
  ui: { fonts: false },
  router: { options: { hashMode: true } },
  css: [
    '@lichess-org/chessground/assets/chessground.base.css',
    '@lichess-org/chessground/assets/chessground.brown.css',
    '~/assets/css/main.css',
  ],
  icon: { clientBundle: { icons: localIcons() } },
  vite: { server: { open: false }, plugins: [voskWorkerPlugin()] },
})
