export default defineNuxtConfig({
  compatibilityDate: '2026-09-29',
  ssr: false,
  modules: ['@pinia/nuxt', '@nuxt/ui'],
  ui: { fonts: false },
  router: { options: { hashMode: true } },
  css: [
    '@lichess-org/chessground/assets/chessground.base.css',
    '@lichess-org/chessground/assets/chessground.brown.css',
    '@lichess-org/chessground/assets/chessground.cburnett.css',
    '~/assets/css/main.css',
  ],
  vite: { server: { open: false } },
})
