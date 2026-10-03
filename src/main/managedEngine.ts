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
import { basename, join } from 'node:path'
import { Readable, Transform } from 'node:stream'
import type { ReadableStream as NodeReadableStream } from 'node:stream/web'
import { pipeline } from 'node:stream/promises'
import { promisify } from 'node:util'
import { pickStockfishAsset, type ReleaseAsset } from './stockfishAsset.ts'

const execFileAsync = promisify(execFile)

/** The Stockfish KChess downloads and owns; the only engine directory KChess ever deletes. */
export const MANAGED_DIR = join(homedir(), '.kchess/engines/stockfish/current')
const EXECUTABLE = process.platform === 'win32' ? 'stockfish.exe' : 'stockfish'
export const MANAGED_PATH = join(MANAGED_DIR, EXECUTABLE)
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
  const path = join(dir, EXECUTABLE)
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
 * Install the latest official native build, verified against the digest GitHub
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
  if (!Array.isArray(release.assets) || typeof release.tag_name !== 'string')
    throw new Error('Invalid Stockfish release metadata.')
  const current = await managedEngine(dir)
  if (current.installed && current.version === release.tag_name)
    return { ...current, version: release.tag_name, updated: false }
  const asset = pickStockfishAsset(release.assets, process.platform, process.arch)
  if (!asset) throw new Error('No compatible official Stockfish download was found.')
  if (
    typeof asset.name !== 'string' ||
    basename(asset.name) !== asset.name ||
    !Number.isFinite(asset.size) ||
    typeof asset.browser_download_url !== 'string'
  )
    throw new Error('Invalid Stockfish download metadata.')
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
    // macOS/Linux tar and Windows' bundled bsdtar both support the official archive formats.
    if (!/\.(zip|tar\.gz|tgz|tar)$/i.test(asset.name))
      throw new Error('Unsupported Stockfish archive.')
    await execFileAsync(
      process.platform === 'win32'
        ? join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe')
        : '/usr/bin/tar',
      ['-xf', archive, '-C', extracted],
      { timeout: 60_000 },
    )
    const source = await findBinary(extracted)
    if (!source) throw new Error('Stockfish executable was missing from the release.')

    await mkdir(dir, { recursive: true })
    const path = join(dir, EXECUTABLE)
    const pending = `${path}.pending`
    await copyFile(source, pending)
    await chmod(pending, 0o755)
    // Verify UCI before replacing the working engine; this catches wrong architecture and broken downloads.
    const probe = execFile(pending, [], { timeout: 15_000, maxBuffer: 1_000_000 }, () => {})
    await new Promise<void>((resolve, reject) => {
      let output = ''
      let settled = false
      const finish = (error?: Error): void => {
        if (settled) return
        settled = true
        probe.kill()
        if (error) reject(error)
        else resolve()
      }
      probe.on('error', (error) => finish(error))
      probe.on('exit', () =>
        finish(
          new Error('The downloaded Stockfish could not start. The previous engine was retained.'),
        ),
      )
      probe.stdout?.on('data', (chunk: Buffer) => {
        output += chunk.toString()
        if (/^uciok\s*$/m.test(output)) finish()
        else if (output.length > 1_000_000) finish(new Error('Invalid Stockfish output.'))
      })
      probe.stdin?.end('uci\n')
    })
    await rename(pending, path)
    await writeFile(join(dir, VERSION_FILE), `${release.tag_name}\n`)
    return { installed: true, path, version: release.tag_name, updated: true }
  } finally {
    await rm(staging, { recursive: true, force: true })
  }
}
