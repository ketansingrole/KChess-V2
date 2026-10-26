import { lstat, readdir, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { getRawHeader } from '@electron/asar'

import { PERFORMANCE_BUDGETS } from './performance-budgets.mjs'
import { verifyNoDevBuild } from './dev-build-marker.mjs'

const MiB = 1024 * 1024
const forbidden =
  /(?:^|\/)(?:[^/]*\.(?:app|framework|asar|dmg|exe|AppImage|deb|zip|blockmap)|\.temp[^/]*|mac(?:-arm64|-x64)?|linux-unpacked|win-unpacked)(?:\/|$)/i

/** The speech recognition runtime: the helper, libvosk, its MinGW runtime (Windows), notices. */
const VOICE_RUNTIME =
  /^node_modules\/@kchess\/native\/voice\/(?:kchess-voice(?:\.exe)?|libvosk\.(?:dylib|so|dll)|lib(?:stdc\+\+-6|gcc_s_seh-1|winpthread-1)\.dll|NOTICE\.txt|APACHE-2\.0\.txt)$/

export function verifyEntries(entries, label = 'package') {
  let total = 0
  let voice = 0
  for (const { path, size } of entries) {
    if (VOICE_RUNTIME.test(path)) {
      voice += size
      continue
    }
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
    throw new Error(
      `${label} payload exceeds the ${PERFORMANCE_BUDGETS.packageBytes / MiB} MiB budget.`,
    )
  if (voice > PERFORMANCE_BUDGETS.voiceRuntimeBytes)
    throw new Error(
      `${label} voice runtime exceeds the ${PERFORMANCE_BUDGETS.voiceRuntimeBytes / MiB} MiB budget.`,
    )
  return total
}

export function verifyRendererEntries(entries) {
  const bytes = verifyEntries(entries, 'Renderer')
  if (bytes > PERFORMANCE_BUDGETS.rendererBytes)
    throw new Error(`Renderer exceeds the ${PERFORMANCE_BUDGETS.rendererBytes / MiB} MiB budget.`)
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
      else {
        entries.push({ path, size: info.size })
        if (/\.(?:js|mjs|html|css|json)$/.test(name))
          await verifyNoDevBuild(join(directory, name), 'Renderer')
      }
    }
  }
  await walk(root)
  const bytes = verifyRendererEntries(entries)
  console.info(`[kchess] Renderer verified: ${(bytes / MiB).toFixed(1)} MiB.`)
}

/** The license and notices a GPL distribution must carry, as app resources. */
export const LEGAL_FILES = ['LICENSE', 'THIRD_PARTY_NOTICES.md']

/** Throws unless every legal file is a non-empty file in `resources`. */
export async function verifyLegalFiles(resources) {
  for (const name of LEGAL_FILES) {
    let info
    try {
      info = await stat(join(resources, name))
    } catch (cause) {
      throw new Error(`Missing ${name} in the package resources.`, { cause })
    }
    if (!info.isFile() || !info.size) throw new Error(`Missing ${name} in the package resources.`)
  }
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
  for (const path of ['out/main/index.js', 'out/preload/index.cjs']) {
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
  // Speech recognition: the helper and libvosk, unpacked so the OS can run and load them.
  const library = { darwin: 'libvosk.dylib', win32: 'libvosk.dll' }[platform] ?? 'libvosk.so'
  for (const name of [platform === 'win32' ? 'kchess-voice.exe' : 'kchess-voice', library]) {
    const path = `node_modules/@kchess/native/voice/${name}`
    if (!entries.some((entry) => entry.path === path))
      throw new Error(`Missing voice runtime: ${path}`)
    if (!(await stat(join(resources, 'app.asar.unpacked', path))).size)
      throw new Error(`Voice runtime is empty: ${path}`)
  }
  await verifyLegalFiles(resources)
  if ((await stat(archive)).size > PERFORMANCE_BUDGETS.packageBytes)
    throw new Error(`app.asar exceeds the ${PERFORMANCE_BUDGETS.packageBytes / MiB} MiB budget.`)
  console.info(
    `[kchess] Package verified: ${(bytes / MiB).toFixed(1)} MiB of app payload (excluding Electron).`,
  )
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  if (!process.argv[2])
    throw new Error('Usage: node tooling/verify-package.mjs <app-output-directory>')
  await verifyPackage(process.argv[2])
}
