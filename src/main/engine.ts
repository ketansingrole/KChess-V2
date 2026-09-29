import { spawn } from 'node:child_process'
import { access, chmod, copyFile, mkdir, mkdtemp, open, readdir, rename, rm, stat } from 'node:fs/promises'
import { constants } from 'node:fs'
import { join } from 'node:path'
import { homedir, tmpdir } from 'node:os'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)

async function findBinary(root: string): Promise<string | null> {
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = join(root, entry.name)
    if (entry.isDirectory()) { const nested = await findBinary(path); if (nested) return nested }
    else if (entry.isFile() && entry.name.toLowerCase().startsWith('stockfish') && !/\.(txt|md|pdf)$/i.test(entry.name)) return path
  }
  return null
}

export async function installStockfish(): Promise<{ path: string; version: string }> {
  if (process.platform !== 'darwin' || process.arch !== 'arm64') throw new Error('Managed Stockfish installation currently supports macOS Apple Silicon.')
  const releaseResponse = await fetch('https://api.github.com/repos/official-stockfish/Stockfish/releases/latest', { headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'KChess-Electron' } })
  if (!releaseResponse.ok) throw new Error(`Could not check Stockfish releases (${releaseResponse.status}).`)
  const release = await releaseResponse.json() as { tag_name: string; assets: Array<{ name: string; browser_download_url: string; size: number }> }
  const asset = release.assets.find(a => /macos.*m1-apple-silicon/i.test(a.name)) ?? release.assets.find(a => /macos.*apple-silicon/i.test(a.name)) ?? release.assets.find(a => /macos.*arm64/i.test(a.name))
  if (!asset) throw new Error('No macOS Apple Silicon Stockfish download was found.')
  const response = await fetch(asset.browser_download_url)
  if (!response.ok || !response.body) throw new Error(`Stockfish download failed (${response.status}).`)
  const staging = await mkdtemp(join(tmpdir(), 'kchess-stockfish-'))
  try {
    const archive = join(staging, asset.name)
    const file = await open(archive, 'w')
    let bytes = 0
    try {
      const reader = response.body.getReader()
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        let offset = 0
        while (offset < value.byteLength) { const result = await file.write(value, offset, value.byteLength - offset); offset += result.bytesWritten }
        bytes += value.byteLength
      }
    } finally { await file.close() }
    if (asset.size && bytes !== asset.size) throw new Error('Stockfish download was incomplete.')
    const extracted = join(staging, 'extracted')
    await mkdir(extracted)
    if (/\.zip$/i.test(asset.name)) await execFileAsync('/usr/bin/unzip', ['-o', archive, '-d', extracted])
    else if (/\.(tar\.gz|tgz)$/i.test(asset.name)) await execFileAsync('/usr/bin/tar', ['-xzf', archive, '-C', extracted])
    else if (/\.tar$/i.test(asset.name)) await execFileAsync('/usr/bin/tar', ['-xf', archive, '-C', extracted])
    else throw new Error(`Unsupported Stockfish archive: ${asset.name}`)
    const source = await findBinary(extracted)
    if (!source) throw new Error('Stockfish executable was missing from the release.')
    const destination = join(homedir(), '.kchess/engines/stockfish/current/stockfish')
    await mkdir(join(homedir(), '.kchess/engines/stockfish/current'), { recursive: true })
    const pending = `${destination}.pending`
    await copyFile(source, pending)
    await chmod(pending, 0o755)
    await rename(pending, destination)
    return { path: destination, version: release.tag_name }
  } finally { await rm(staging, { recursive: true, force: true }) }
}

export async function engineStatus(configured = ''): Promise<{ path: string; ready: boolean }> {
  const candidates = [configured, join(homedir(), '.kchess/engines/stockfish/current/stockfish'), '/opt/homebrew/bin/stockfish', '/usr/local/bin/stockfish', '/usr/bin/stockfish'].filter(Boolean)
  for (const path of candidates) {
    try { await access(path, constants.X_OK); if ((await stat(path)).isFile()) return { path, ready: true } } catch { /* try next */ }
  }
  return { path: configured, ready: false }
}

export async function bestMove(moves: string[], level: 'low' | 'medium' | 'high', configured = ''): Promise<string> {
  const status = await engineStatus(configured)
  if (!status.ready) throw new Error('Stockfish is not installed. Set its path in Settings.')
  const profile = { low: { elo: 1350, time: 700 }, medium: { elo: 1800, time: 1400 }, high: { elo: 0, time: 2500 } }[level]
  return new Promise((resolve, reject) => {
    const child = spawn(status.path, [], { stdio: ['pipe', 'pipe', 'pipe'] })
    let buffer = ''
    let phase: 'uci' | 'ready' | 'search' = 'uci'
    let settled = false
    const finish = (error?: Error, move?: string): void => {
      if (settled) return
      settled = true; clearTimeout(timeout); child.kill()
      if (error) reject(error)
      else resolve(move ?? '')
    }
    const timeout = setTimeout(() => finish(new Error('Stockfish timed out.')), 15_000)
    child.on('error', error => finish(error))
    child.on('exit', code => { if (!settled) finish(new Error(`Stockfish exited (${code}).`)) })
    child.stdout.setEncoding('utf8')
    child.stdout.on('data', (chunk: string) => {
      buffer += chunk
      let end = buffer.indexOf('\n')
      while (end >= 0) {
        const line = buffer.slice(0, end).trim()
        buffer = buffer.slice(end + 1)
        if (phase === 'uci' && line === 'uciok') {
          if (profile.elo) {
            child.stdin.write('setoption name UCI_LimitStrength value true\n')
            child.stdin.write(`setoption name UCI_Elo value ${profile.elo}\n`)
          } else child.stdin.write('setoption name UCI_LimitStrength value false\n')
          child.stdin.write('isready\n'); phase = 'ready'
        } else if (phase === 'ready' && line === 'readyok') {
          child.stdin.write(`position startpos moves ${moves.join(' ')}\n`)
          child.stdin.write(`go movetime ${profile.time}\n`); phase = 'search'
        } else if (phase === 'search' && line.startsWith('bestmove ')) {
          const move = line.split(/\s+/)[1]
          if (!move || move === '(none)') finish(new Error('Stockfish found no legal move.'))
          else finish(undefined, move)
        }
        end = buffer.indexOf('\n')
      }
    })
    child.stdin.write('uci\n')
  })
}
