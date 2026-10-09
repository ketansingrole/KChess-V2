import type { Plugin } from 'vite'
import { readdirSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { join } from 'node:path'

/** Include icons in data arrays/conditional props that the template scanner cannot discover. */
export function localIcons(): string[] {
  const root = fileURLToPath(new URL('../apps/desktop/app', import.meta.url))
  const icons = new Set<string>()
  for (const path of readdirSync(root, { recursive: true, encoding: 'utf8' })) {
    if (!/\.(ts|vue)$/.test(path)) continue
    for (const match of readFileSync(join(root, path), 'utf8').matchAll(/i-lucide-([a-z0-9-]+)/g))
      icons.add(`lucide:${match[1]}`)
  }
  return [...icons].sort()
}

/** Extract the pinned library's embedded worker so only that worker needs JS evaluation. */
export function voskWorkerPlugin(): Plugin {
  return {
    name: 'kchess-local-vosk-worker',
    apply: 'build',
    enforce: 'pre',
    transform(code, id) {
      if (!id.endsWith('/vosk-browser/dist/vosk.js')) return
      const prefix = "var WorkerFactory = createBase64WorkerFactory('"
      const suffix = "', null, false);"
      const start = code.indexOf(prefix)
      const end = code.indexOf(suffix, start + prefix.length)
      if (start < 0 || end < 0)
        throw new Error('Vosk worker format changed; review its extraction and CSP.')
      const decoded = Buffer.from(code.slice(start + prefix.length, end), 'base64').toString('utf8')
      const source = decoded.slice(decoded.indexOf('\n', 10) + 1)
      const reference = this.emitFile({
        type: 'asset',
        fileName: '_nuxt/vosk-worker.js',
        source,
      })
      return {
        code:
          code.slice(0, start) +
          `var WorkerFactory = function(options) { return new Worker(import.meta.ROLLUP_FILE_URL_${reference}, options); };` +
          code.slice(end + suffix.length),
        map: null,
      }
    },
  }
}
