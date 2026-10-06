import { beforeEach, describe, expect, it, vi } from 'vitest'
import { handleAppProtocol, registerAppScheme } from '../../src/main/appProtocol'

const electronMock = vi.hoisted(() => ({
  handler: null as ((request: { url: string; method: string }) => Promise<Response>) | null,
  privileged: null as { scheme: string; privileges: Record<string, boolean> }[] | null,
  fetch: vi.fn(async () => new Response('asset')),
}))

vi.mock('electron', () => ({
  app: { getAppPath: () => process.cwd() },
  protocol: {
    registerSchemesAsPrivileged: (schemes: unknown) => {
      electronMock.privileged = schemes as never
    },
    handle: (_scheme: string, handler: never) => {
      electronMock.handler = handler as never
    },
  },
  net: { fetch: (...args: unknown[]) => electronMock.fetch(...(args as [])) },
}))

const get = (url: string, method = 'GET') => electronMock.handler!({ url, method })

beforeEach(() => {
  electronMock.handler = null
  electronMock.privileged = null
  electronMock.fetch.mockReset()
  electronMock.fetch.mockResolvedValue(new Response('asset'))
})

describe('app protocol', () => {
  it('registers kchess as a secure standard scheme', () => {
    registerAppScheme()
    expect(electronMock.privileged).toMatchObject([
      { scheme: 'kchess', privileges: expect.objectContaining({ secure: true, standard: true }) },
    ])
  })

  it('rejects non-GET, foreign hosts and path traversal', async () => {
    handleAppProtocol('/tmp/model.tar.gz')
    expect(await get('kchess://app/index.html', 'POST')).toMatchObject({ status: 403 })
    expect(await get('kchess://evil/index.html')).toMatchObject({ status: 403 })
    expect(await get('kchess://app/..%2Fsecret')).toMatchObject({ status: 403 })
  })

  it('serves local files with a content-security-policy header', async () => {
    handleAppProtocol('/tmp/model.tar.gz')
    const response = await get('kchess://app/index.html')
    expect(response.status).toBe(200)
    expect(response.headers.get('Content-Security-Policy')).toContain('script-src')
  })

  it('serves the voice model as gzip and maps fetch failures to 404', async () => {
    handleAppProtocol('/tmp/model.tar.gz')
    const model = await get('kchess://app/voice/model.tar.gz')
    expect(model.status).toBe(200)
    expect(model.headers.get('Content-Type')).toBe('application/gzip')
    electronMock.fetch.mockRejectedValueOnce(new Error('gone'))
    expect(await get('kchess://app/voice/model.tar.gz')).toMatchObject({ status: 404 })
  })
})
