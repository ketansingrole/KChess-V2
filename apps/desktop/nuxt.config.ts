import { localIcons, voskWorkerPlugin } from '../../tooling/vosk-worker-plugin.ts'
import { rendererCoreAliases } from '../../tooling/core-aliases.ts'

export default defineNuxtConfig({
  compatibilityDate: '2026-09-29',
  future: { compatibilityVersion: 5 },
  ssr: false,
  alias: rendererCoreAliases,
  // Nitro leaves the Nuxt renderer external on Windows (backslash paths miss
  // its `nuxt/dist` inline rule), so every prerendered route 500s with
  // "Either manifest or precomputed data must be provided"
  // (https://github.com/nuxt/nuxt/issues/36467). Inline it with a
  // separator-agnostic pattern until the pinned Nuxt carries the fix.
  nitro: {
    // Ignore model files left by older builds; voice models now live in the user's cache.
    ignore: ['**/voice/model.tar.gz*'],
    externals: { inline: [/[\\/]node_modules[\\/]nuxt[\\/]dist[\\/]/] },
  },
  modules: ['@pinia/nuxt', '@nuxt/ui', '@nuxt/eslint'],
  ui: { fonts: false, experimental: { componentDetection: true } },
  router: { options: { hashMode: true } },
  css: [
    '@lichess-org/chessground/assets/chessground.base.css',
    '@lichess-org/chessground/assets/chessground.brown.css',
    '~/assets/css/main.css',
  ],
  icon: { clientBundle: { icons: localIcons() } },
  vite: { server: { open: false }, plugins: [voskWorkerPlugin()] },
})
