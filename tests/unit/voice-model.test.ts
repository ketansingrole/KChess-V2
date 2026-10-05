import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createHash } from 'node:crypto'
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { zipSync } from 'fflate'
import { VoiceModelCache } from '../../src/main/voiceModel'

let directory: string
const zip = zipSync({ 'test-model/am/final.mdl': new TextEncoder().encode('test model') })
const source = {
  name: 'test-model',
  url: 'https://example.test/model.zip',
  sha256: createHash('sha256').update(zip).digest('hex'),
}
const download = () => vi.fn<typeof fetch>(async () => new Response(Buffer.from(zip)))

beforeEach(async () => {
  directory = await mkdtemp(join(tmpdir(), 'kchess-voice-test-'))
})
afterEach(async () => {
  await rm(directory, { recursive: true, force: true })
})

describe('voice model cache', () => {
  it('checks installation without downloading and rejects a damaged local cache', async () => {
    const fetchModel = download()
    const cache = new VoiceModelCache(directory, fetchModel, source)
    expect(await cache.status()).toEqual({ installed: false, bytes: 0, busy: false })
    expect(fetchModel).not.toHaveBeenCalled()
    await cache.ensure()
    const archive = await readFile(cache.path)
    expect(await cache.status()).toEqual({ installed: true, bytes: archive.length, busy: false })
    expect(fetchModel).toHaveBeenCalledOnce()
    archive[archive.length - 1] ^= 1
    await writeFile(cache.path, archive)
    expect(await cache.status()).toEqual({ installed: false, bytes: 0, busy: false })
    expect(fetchModel).toHaveBeenCalledOnce()
  })

  it('reports shared preparation while a download is in flight', async () => {
    let respond!: (response: Response) => void
    const fetchModel = vi.fn<typeof fetch>(
      () =>
        new Promise<Response>((resolve) => {
          respond = resolve
        }),
    )
    const cache = new VoiceModelCache(directory, fetchModel, source)
    const pending = cache.ensure()
    await vi.waitFor(() => expect(fetchModel).toHaveBeenCalledOnce())
    expect(await cache.status()).toMatchObject({
      busy: true,
      installed: false,
      progress: { phase: 'downloading' },
    })
    respond(new Response(Buffer.from(zip)))
    await pending
    expect(await cache.status()).toMatchObject({ busy: false, installed: true })
  })

  it('shares concurrent downloads and reuses a verified cache offline in a new instance', async () => {
    const fetchModel = download()
    const cache = new VoiceModelCache(directory, fetchModel, source)
    const progress = vi.fn()
    const [first, second] = await Promise.all([cache.ensure(progress), cache.ensure()])
    expect(first).toBe(second)
    expect(fetchModel).toHaveBeenCalledOnce()
    expect(progress).toHaveBeenCalledWith(expect.objectContaining({ phase: 'downloading' }))
    expect(progress).toHaveBeenCalledWith(expect.objectContaining({ phase: 'preparing' }))
    expect((await readFile(first)).subarray(0, 2)).toEqual(Buffer.from([0x1f, 0x8b]))
    const offline = vi.fn<typeof fetch>(async () => {
      throw new Error('offline')
    })
    await expect(new VoiceModelCache(directory, offline, source).ensure()).resolves.toBe(first)
    expect(offline).not.toHaveBeenCalled()
    expect(await readdir(directory)).toEqual(
      expect.arrayContaining(['model.tar.gz', 'model.tar.gz.json']),
    )
    expect((await readdir(directory)).some((name) => name.startsWith('.prepare-'))).toBe(false)
  })

  it('rejects a bad checksum without publishing a cache, then permits retry', async () => {
    const fetchModel = download()
    fetchModel.mockResolvedValueOnce(new Response('corrupted download'))
    const cache = new VoiceModelCache(directory, fetchModel, source)
    await expect(cache.ensure()).rejects.toThrow('checksum mismatch')
    expect(await readdir(directory)).toEqual([])
    await expect(cache.ensure()).resolves.toBe(cache.path)
    expect(fetchModel).toHaveBeenCalledTimes(2)
  })

  it('detects same-size cache corruption and downloads a replacement', async () => {
    const fetchModel = download()
    const cache = new VoiceModelCache(directory, fetchModel, source)
    const path = await cache.ensure()
    const original = await readFile(path)
    const damaged = Buffer.from(original)
    damaged[damaged.length - 1] ^= 1
    await writeFile(path, damaged)
    await cache.ensure()
    expect(fetchModel).toHaveBeenCalledTimes(2)
    expect(await readFile(path)).not.toEqual(damaged)
    await cache.ensure()
    expect(fetchModel).toHaveBeenCalledTimes(2)
  })

  it('cleans up failed and interrupted responses and allows another attempt', async () => {
    const fetchModel = download()
    fetchModel.mockResolvedValueOnce(new Response(null, { status: 503 }))
    fetchModel.mockResolvedValueOnce(
      new Response(
        new ReadableStream({
          start(controller) {
            controller.error(new Error('connection interrupted'))
          },
        }),
      ),
    )
    const cache = new VoiceModelCache(directory, fetchModel, source)
    await expect(cache.ensure()).rejects.toThrow('HTTP 503')
    await expect(cache.ensure()).rejects.toThrow('connection interrupted')
    expect(await readdir(directory)).toEqual([])
    await expect(cache.ensure()).resolves.toBe(cache.path)
  })

  it('rejects paths outside the expected model directory even with a matching digest', async () => {
    const unsafe = zipSync({ '../escape': new Uint8Array([1]) })
    const fetchModel = vi.fn<typeof fetch>(async () => new Response(Buffer.from(unsafe)))
    const cache = new VoiceModelCache(directory, fetchModel, {
      ...source,
      sha256: createHash('sha256').update(unsafe).digest('hex'),
    })
    await expect(cache.ensure()).rejects.toThrow('Invalid voice model archive')
    expect(await readdir(directory)).toEqual([])
  })
})

it('keeps the main event loop responsive while preparing a large model', async () => {
  const large = zipSync({ 'test-model/am/final.mdl': new Uint8Array(32 * 1024 * 1024).fill(42) })
  const cache = new VoiceModelCache(
    directory,
    vi.fn(async () => new Response(Buffer.from(large))),
    {
      ...source,
      sha256: createHash('sha256').update(large).digest('hex'),
    },
  )
  let preparing = false,
    ticks = 0
  const timer = setInterval(() => {
    if (preparing) ticks++
  }, 5)
  try {
    await cache.ensure((progress) => {
      preparing = progress.phase === 'preparing'
    })
    expect(ticks).toBeGreaterThan(2)
  } finally {
    clearInterval(timer)
  }
})
