import { execFile } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createWriteStream } from 'node:fs'
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rename,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises'
import { homedir, tmpdir } from 'node:os'
import { join } from 'node:path'
import { Readable, Transform } from 'node:stream'
import type { ReadableStream as NodeReadableStream } from 'node:stream/web'
import { pipeline } from 'node:stream/promises'
import { promisify } from 'node:util'
import { pickMacAsset, type ReleaseAsset } from './stockfishAsset.ts'

const execFileAsync = promisify(execFile)

/** The Stockfish KChess downloads and owns; the only engine directory KChess ever deletes. */
export const MANAGED_DIR = join(homedir(), '.kchess/engines/stockfish/current')
export const MANAGED_PATH = join(MANAGED_DIR, 'stockfish')
const VERSION_FILE = 'VERSION'
const RELEASE_URL = 'https://api.github.com/repos/official-stockfish/Stockfish/releases/latest'
const MAX_DOWNLOAD_BYTES = 300 * 1024 * 1024

export interface ManagedEngine {
  installed: boolean
  path: string
  /** Release tag recorded when it was downloaded, e.g. `sf_17.1`. */
  version?: string
}

export interface ManagedLocation {
  /** Directory holding the downloaded engine. Tests point this at a temp folder. */
  dir?: string
  /** GitHub "latest release" endpoint. Tests point this at a local server. */
  releaseUrl?: string
}

export async function managedEngine(dir = MANAGED_DIR): Promise<ManagedEngine> {
  const path = join(dir, 'stockfish')
  try {
    if (!(await stat(path)).isFile()) return { installed: false, path }
  } catch {
    return { installed: false, path }
  }
  const version = await readFile(join(dir, VERSION_FILE), 'utf8').then(
    (text) => text.trim() || undefined,
    () => undefined,
  )
  return { installed: true, path, version }
}

/** Remove the downloaded engine. Bundled and user-picked engines are never touched. */
export async function deleteManagedEngine(dir = MANAGED_DIR): Promise<void> {
  await rm(dir, { recursive: true, force: true })
}

async function findBinary(root: string): Promise<string | null> {
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = join(root, entry.name)
    if (entry.isDirectory()) {
      const nested = await findBinary(path)
      if (nested) return nested
    } else if (
      entry.isFile() &&
      entry.name.toLowerCase().startsWith('stockfish') &&
      !/\.(txt|md|pdf)$/i.test(entry.name)
    )
      return path
  }
  return null
}

export interface InstallResult extends ManagedEngine {
  version: string
  /** False when the installed engine already matched the latest release and nothing was downloaded. */
  updated: boolean
}

/**
 * Install the latest official macOS build, verified against the digest GitHub
 * publishes. Checks the latest release first and downloads nothing when the
 * installed engine is already that version.
 */
export async function installManagedEngine(location: ManagedLocation = {}): Promise<InstallResult> {
  const dir = location.dir ?? MANAGED_DIR
  const releaseResponse = await fetch(location.releaseUrl ?? RELEASE_URL, {
    headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'KChess-Electron' },
    signal: AbortSignal.timeout(30_000),
  })
  if (!releaseResponse.ok)
    throw new Error(`Could not check Stockfish releases (${releaseResponse.status}).`)
  const release = (await releaseResponse.json()) as { tag_name: string; assets: ReleaseAsset[] }
  const current = await managedEngine(dir)
  if (current.installed && current.version === release.tag_name)
    return { ...current, version: release.tag_name, updated: false }
  const asset = pickMacAsset(release.assets, process.arch)
  if (!asset) throw new Error('No macOS Stockfish download was found.')
  const expected = /^sha256:([0-9a-f]{64})$/i.exec(asset.digest ?? '')?.[1]?.toLowerCase()
  if (!expected)
    throw new Error('The Stockfish release has no SHA-256 digest, so it cannot be verified.')
  if (asset.size > MAX_DOWNLOAD_BYTES) throw new Error('Stockfish download is unexpectedly large.')
  const response = await fetch(asset.browser_download_url, { signal: AbortSignal.timeout(300_000) })
  if (!response.ok || !response.body)
    throw new Error(`Stockfish download failed (${response.status}).`)

  const staging = await mkdtemp(join(tmpdir(), 'kchess-stockfish-'))
  try {
    const archive = join(staging, asset.name)
    const hash = createHash('sha256')
    let bytes = 0
    await pipeline(
      Readable.fromWeb(response.body as NodeReadableStream<Uint8Array>),
      new Transform({
        transform(chunk: Buffer, _encoding, done) {
          bytes += chunk.length
          if (bytes > MAX_DOWNLOAD_BYTES)
            return done(new Error('Stockfish download is unexpectedly large.'))
          hash.update(chunk)
          done(null, chunk)
        },
      }),
      createWriteStream(archive),
    )
    if (asset.size && bytes !== asset.size) throw new Error('Stockfish download was incomplete.')
    if (hash.digest('hex') !== expected)
      throw new Error('Stockfish download failed its SHA-256 check.')

    const extracted = join(staging, 'extracted')
    await mkdir(extracted)
    if (/\.zip$/i.test(asset.name))
      await execFileAsync('/usr/bin/unzip', ['-q', archive, '-d', extracted])
    else if (/\.(tar\.gz|tgz)$/i.test(asset.name))
      await execFileAsync('/usr/bin/tar', ['-xzf', archive, '-C', extracted])
    else if (/\.tar$/i.test(asset.name))
      await execFileAsync('/usr/bin/tar', ['-xf', archive, '-C', extracted])
    else throw new Error(`Unsupported Stockfish archive: ${asset.name}`)
    const source = await findBinary(extracted)
    if (!source) throw new Error('Stockfish executable was missing from the release.')

    await mkdir(dir, { recursive: true })
    const path = join(dir, 'stockfish')
    const pending = `${path}.pending`
    await copyFile(source, pending)
    await chmod(pending, 0o755)
    await rename(pending, path)
    await writeFile(join(dir, VERSION_FILE), `${release.tag_name}\n`)
    return { installed: true, path, version: release.tag_name, updated: true }
  } finally {
    await rm(staging, { recursive: true, force: true })
  }
}
