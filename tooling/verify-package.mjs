import { lstat, readdir, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { getRawHeader } from '@electron/asar'

import { PERFORMANCE_BUDGETS } from './performance-budgets.mjs'

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
      if (
        path.startsWith('node_modules/@kchess/native/') &&
        !/^node_modules\/@kchess\/native\/(?:package\.json|index\.cjs|THIRD_PARTY_LICENSES\.txt|kchess-native\.[a-z0-9]+-[a-z0-9]+\.node)$/.test(
          path,
        )
      )
        throw new Error(`${label} contains native rules sources or extra files: ${path}`)
    }
    if (size > PERFORMANCE_BUDGETS.individualFileBytes)
      throw new Error(`${label} contains an unexpectedly large file: ${path}`)
    total += size
  }
  if (total > PERFORMANCE_BUDGETS.packageBytes)
    throw new Error(`${label} payload exceeds the 32 MiB budget.`)
  return total
}

export function verifyRendererEntries(entries) {
  const bytes = verifyEntries(entries, 'Renderer')
  if (bytes > PERFORMANCE_BUDGETS.rendererBytes)
    throw new Error('Renderer exceeds the 12 MiB budget.')
  return bytes
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
  const bytes = verifyRendererEntries(entries)
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
  for (const path of [
    'out/main/index.js',
    'out/preload/index.cjs',
    '.output/public/_nuxt/vosk-worker.js',
  ]) {
    if (!entries.some((entry) => entry.path === path && entry.size > 0))
      throw new Error(`Missing desktop service: ${path}`)
  }
  for (const file of ['js', 'wasm']) {
    const path = `node_modules/stockfish/bin/stockfish-19-lite.${file}`
    if (!entries.some((entry) => entry.path === path))
      throw new Error(`Missing bundled engine: ${path}`)
    if (!(await stat(join(resources, 'app.asar.unpacked', path))).size)
      throw new Error(`Bundled engine is empty: ${path}`)
  }
  // The Rust rules for this platform, unpacked so the OS can load the module.
  const native = entries.find((entry) =>
    new RegExp(`^node_modules/@kchess/native/kchess-native\\.${platform}-[a-z0-9]+\\.node$`).test(
      entry.path,
    ),
  )
  if (!native) throw new Error(`Missing native rules for ${platform}.`)
  if (
    !entries.some((entry) => entry.path === 'node_modules/@kchess/native/THIRD_PARTY_LICENSES.txt')
  )
    throw new Error('Missing license texts for the Rust crates in the native rules.')
  if (!(await stat(join(resources, 'app.asar.unpacked', native.path))).size)
    throw new Error(`Native rules are empty: ${native.path}`)
  if ((await stat(archive)).size > 32 * MiB) throw new Error('app.asar exceeds the 32 MiB budget.')
  console.info(
    `[kchess] Package verified: ${(bytes / MiB).toFixed(1)} MiB of app payload (excluding Electron).`,
  )
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  if (!process.argv[2])
    throw new Error('Usage: node tooling/verify-package.mjs <app-output-directory>')
  await verifyPackage(process.argv[2])
}
