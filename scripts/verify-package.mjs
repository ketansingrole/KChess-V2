import { lstat, readdir, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { getRawHeader } from '@electron/asar'

const MiB = 1024 * 1024
const forbidden =
  /(?:^|\/)(?:[^/]*\.(?:app|framework|asar|dmg|exe|AppImage|deb|zip|blockmap)|\.temp[^/]*|mac(?:-arm64|-x64)?|linux-unpacked|win-unpacked)(?:\/|$)/i

export function verifyEntries(entries, label = 'package') {
  let total = 0
  for (const { path, size } of entries) {
    if (forbidden.test(path) || /(?:^|\/)voice\/model\.tar\.gz/.test(path))
      throw new Error(`${label} contains a build artifact or bundled voice model: ${path}`)
    if (path.startsWith('node_modules/')) {
      const name = path.split('/')[1]
      if (['@pinia', '@nuxt', 'nuxt', 'pinia', 'vosk-browser'].includes(name))
        throw new Error(`${label} contains a redundant renderer/build dependency: ${path}`)
      if (
        path.startsWith('node_modules/stockfish/bin/') &&
        !/^node_modules\/stockfish\/bin\/stockfish-19-lite\.(?:js|wasm)$/.test(path)
      )
        throw new Error(`${label} contains an unused Stockfish build: ${path}`)
    }
    if (size > 16 * MiB) throw new Error(`${label} contains an unexpectedly large file: ${path}`)
    total += size
  }
  if (total > 32 * MiB) throw new Error(`${label} payload exceeds the 32 MiB budget.`)
  return total
}

export async function verifyRenderer(root) {
  const entries = []
  async function walk(directory, prefix = '') {
    for (const name of await readdir(directory)) {
      const path = prefix + name
      const info = await lstat(join(directory, name))
      if (info.isSymbolicLink()) throw new Error(`Renderer contains a symlink: ${path}`)
      if (info.isDirectory()) await walk(join(directory, name), `${path}/`)
      else entries.push({ path, size: info.size })
    }
  }
  await walk(root)
  const bytes = verifyEntries(entries, 'Renderer')
  console.info(`[kchess] Renderer verified: ${(bytes / MiB).toFixed(1)} MiB.`)
}

export async function verifyPackage(appOutDir, platform = process.platform) {
  const resources =
    platform === 'darwin'
      ? join(appOutDir, 'KChess.app', 'Contents', 'Resources')
      : join(appOutDir, 'resources')
  const archive = join(resources, 'app.asar')
  const entries = []
  function walk(files, prefix = '') {
    for (const [name, info] of Object.entries(files)) {
      const path = prefix + name
      if (info.files) walk(info.files, `${path}/`)
      else entries.push({ path, size: info.size || 0 })
    }
  }
  walk(getRawHeader(archive).header.files)
  const bytes = verifyEntries(entries)
  for (const file of ['js', 'wasm']) {
    const path = `node_modules/stockfish/bin/stockfish-19-lite.${file}`
    if (!entries.some((entry) => entry.path === path))
      throw new Error(`Missing bundled engine: ${path}`)
    if (!(await stat(join(resources, 'app.asar.unpacked', path))).size)
      throw new Error(`Bundled engine is empty: ${path}`)
  }
  if ((await stat(archive)).size > 32 * MiB) throw new Error('app.asar exceeds the 32 MiB budget.')
  console.info(
    `[kchess] Package verified: ${(bytes / MiB).toFixed(1)} MiB of app payload (excluding Electron).`,
  )
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  if (!process.argv[2])
    throw new Error('Usage: node scripts/verify-package.mjs <app-output-directory>')
  await verifyPackage(process.argv[2])
}
