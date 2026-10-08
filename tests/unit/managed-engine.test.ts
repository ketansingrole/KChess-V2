import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mkdtemp, rm, writeFile, mkdir, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  deleteManagedEngine,
  installManagedEngine,
  managedEngine,
} from '../../src/core/managedEngine'

const exe = process.platform === 'win32' ? 'stockfish.exe' : 'stockfish'
let dir: string

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'kchess-engine-'))
})

afterEach(async () => {
  await rm(dir, { recursive: true, force: true })
})

describe('managed engine install state', () => {
  it('reports missing when the directory or binary is absent', async () => {
    expect(await managedEngine(join(dir, 'empty'))).toEqual({
      installed: false,
      path: join(dir, 'empty', exe),
    })
    await mkdir(join(dir, 'notadir', 'sub'), { recursive: true })
    expect((await managedEngine(join(dir, 'notadir', 'sub'))).installed).toBe(false)
  })

  it('reports the recorded version, and no version when the file is missing', async () => {
    await writeFile(join(dir, exe), '#!/bin/sh\nexit 0\n')
    expect(await managedEngine(dir)).toMatchObject({ installed: true, version: undefined })
    await writeFile(join(dir, 'VERSION'), 'sf_17.1\n')
    expect(await managedEngine(dir)).toMatchObject({ installed: true, version: 'sf_17.1' })
  })

  it('deletes only its own directory, optionally under engine maintenance', async () => {
    await writeFile(join(dir, exe), 'binary')
    let maintained = false
    await deleteManagedEngine(dir, async (commit) => {
      maintained = true
      await commit()
    })
    expect(maintained).toBe(true)
    await expect(stat(dir)).rejects.toThrow()
  })

  it('skips the download when the installed engine already matches the release', async () => {
    await writeFile(join(dir, exe), 'binary')
    await writeFile(join(dir, 'VERSION'), 'sf_17.1\n')
    const release = vi.fn(
      async () => new Response(JSON.stringify({ tag_name: 'sf_17.1', assets: [] })),
    )
    const download = vi.fn(async () => new Response('unused'))
    vi.stubGlobal('fetch', async (url: unknown) =>
      String(url).includes('api.github.com') ? release() : download(),
    )
    const result = await installManagedEngine({ dir, releaseUrl: 'https://api.github.com/x' })
    expect(result).toMatchObject({ installed: true, version: 'sf_17.1', updated: false })
    expect(release).toHaveBeenCalledOnce()
    expect(download).not.toHaveBeenCalled()
  })

  it('rejects invalid release metadata instead of downloading', async () => {
    vi.stubGlobal('fetch', async () => new Response(JSON.stringify({ nope: true })))
    await expect(
      installManagedEngine({ dir, releaseUrl: 'https://example.test/r' }),
    ).rejects.toThrow(/metadata|releases/i)
  })
})
