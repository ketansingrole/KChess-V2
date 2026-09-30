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

export function handleAppProtocol(): void {
  const root = resolve(join(app.getAppPath(), '.output', 'public'))
  protocol.handle('kchess', async (request) => {
    const url = new URL(request.url)
    if (url.host !== 'app' || request.method !== 'GET') return new Response(null, { status: 403 })
    let path: string
    try {
      path = resolve(
        root,
        `.${decodeURIComponent(url.pathname === '/' ? '/index.html' : url.pathname)}`,
      )
    } catch {
      return new Response(null, { status: 400 })
    }
    if (!path.startsWith(`${root}${sep}`)) return new Response(null, { status: 403 })
    try {
      return await net.fetch(pathToFileURL(path).href)
    } catch {
      return new Response(null, { status: 404 })
    }
  })
}
