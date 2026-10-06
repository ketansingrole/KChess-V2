import { createHash } from 'node:crypto'
import { APP_CSP, VOICE_WORKER_CSP } from './appOrigin'
import { logDebug } from './logger'
import { app, net, protocol } from 'electron'
import { join, resolve, sep } from 'node:path'
import { pathToFileURL } from 'node:url'

// A secure origin lets packaged Web Workers use IndexedDB and load local audio/model assets.
export function registerAppScheme(): void {
  protocol.registerSchemesAsPrivileged([
    {
      scheme: 'kchess',
      privileges: {
        standard: true,
        secure: true,
        supportFetchAPI: true,
        corsEnabled: true,
        stream: true,
      },
    },
  ])
}

export function handleAppProtocol(voiceModelPath: string): void {
  const root = resolve(join(app.getAppPath(), '.output', 'public'))
  protocol.handle('kchess', async (request) => {
    const url = new URL(request.url)
    if (url.host !== 'app' || request.method !== 'GET') return new Response(null, { status: 403 })
    if (url.pathname === '/voice/model.tar.gz') {
      try {
        const response = await net.fetch(pathToFileURL(voiceModelPath).href)
        // The same local asset is used by the localhost renderer during development.
        return new Response(response.body, {
          status: response.status,
          headers: { 'Content-Type': 'application/gzip', 'Access-Control-Allow-Origin': '*' },
        })
      } catch (cause) {
        logDebug('startup', 'Voice model asset is unavailable:', cause)
        return new Response(null, { status: 404 })
      }
    }
    let path: string
    try {
      path = resolve(
        root,
        `.${decodeURIComponent(url.pathname === '/' ? '/index.html' : url.pathname)}`,
      )
    } catch (cause) {
      logDebug('startup', 'Invalid app protocol path:', cause)
      return new Response(null, { status: 400 })
    }
    if (!path.startsWith(`${root}${sep}`)) return new Response(null, { status: 403 })
    try {
      const response = await net.fetch(pathToFileURL(path).href)
      const headers = new Headers(response.headers)
      if (path.endsWith('.html')) {
        const html = await response.text()
        // Only exact generated inline scripts/import maps are permitted, never arbitrary inline code.
        const hashes = [...html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/gi)]
          .filter((match) => match[1])
          .map((match) => `'sha256-${createHash('sha256').update(match[1]!).digest('base64')}'`)
        headers.set(
          'Content-Security-Policy',
          APP_CSP.replace("script-src 'self'", `script-src 'self' ${hashes.join(' ')}`),
        )
        return new Response(html, { status: response.status, headers })
      }
      headers.set(
        'Content-Security-Policy',
        url.pathname === '/_nuxt/vosk-worker.js' ? VOICE_WORKER_CSP : APP_CSP,
      )
      return new Response(response.body, { status: response.status, headers })
    } catch (cause) {
      logDebug('startup', 'App protocol asset is unavailable:', cause)
      return new Response(null, { status: 404 })
    }
  })
}
