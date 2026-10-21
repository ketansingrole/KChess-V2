import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { inflateRawSync } from 'node:zlib'

/**
 * The voice runtime the core runs for speech recognition (`crates/kchess-voice`): the helper,
 * the official prebuilt libvosk beside it and their notices, staged in
 * crates/kchess-node/voice/ for development, `@kchess/native` and packaging.
 *
 * libvosk comes from the Vosk project's own release archives, pinned by SHA-256 and cached in
 * .data/vosk/. A digest mismatch stops the build. The newest prebuilt libraries are 0.3.45
 * (Linux and Windows, Dec 2022) and 0.3.42 (macOS, May 2022; the 0.3.44 macOS wheel holds the
 * same bytes).
 */
const root = fileURLToPath(new URL('..', import.meta.url))
const RELEASES = 'https://github.com/alphacep/vosk-api/releases/download'

const PINS = {
  darwin: {
    url: `${RELEASES}/v0.3.42/vosk-osx-0.3.42.zip`,
    sha256: '65395f196c9d0583d79949142b25560acaf9c295f36284e18433097f3adb0ea1',
    files: { 'vosk-osx-0.3.42/libvosk.dylib': 'libvosk.dylib' },
  },
  linux: {
    url: `${RELEASES}/v0.3.45/vosk-linux-x86_64-0.3.45.zip`,
    sha256: 'bbdc8ed85c43979f6443142889770ea95cbfbc56cffb5c5dcd73afa875c5fbb2',
    files: { 'vosk-linux-x86_64-0.3.45/libvosk.so': 'libvosk.so' },
  },
  win32: {
    url: `${RELEASES}/v0.3.45/vosk-win64-0.3.45.zip`,
    sha256: 'f1dcc9cca460630f81ea8f71794f69c80bed6556d2a4e6237b5785e1d2dff34b',
    // libvosk is built with MinGW and needs its C++ runtime beside it.
    files: Object.fromEntries(
      ['libvosk.dll', 'libstdc++-6.dll', 'libgcc_s_seh-1.dll', 'libwinpthread-1.dll'].map(
        (name) => [`vosk-win64-0.3.45/${name}`, name],
      ),
    ),
  },
}

/** The pin for this host. Linux and Windows releases are x64 only, as KChess ships them. */
export function voicePin(platform = process.platform, arch = process.arch) {
  const pin = PINS[platform]
  if (!pin || (platform !== 'darwin' && arch !== 'x64'))
    throw new Error(`No voice runtime for ${platform}-${arch}.`)
  return pin
}

const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex')

/** The pinned archive, from the cache or downloaded once; its digest is always checked. */
async function archive(pin) {
  const cache = join(root, '.data', 'vosk', pin.url.split('/').pop())
  let bytes = existsSync(cache) ? readFileSync(cache) : undefined
  if (!bytes || sha256(bytes) !== pin.sha256) {
    console.info(`[kchess] Downloading ${pin.url}…`)
    const response = await fetch(pin.url)
    if (!response.ok) throw new Error(`Voice runtime download failed: HTTP ${response.status}.`)
    bytes = Buffer.from(await response.arrayBuffer())
    if (sha256(bytes) !== pin.sha256)
      throw new Error(`Voice runtime checksum mismatch for ${pin.url}.`)
    mkdirSync(join(root, '.data', 'vosk'), { recursive: true })
    writeFileSync(cache, bytes)
  }
  return bytes
}

/**
 * The named members of a zip (stored or deflated, no zip64), read through its central
 * directory. Only digest-checked archives reach this.
 */
export function unzipMembers(zip, names) {
  const wanted = new Set(names)
  let end = -1
  for (let at = zip.length - 22; at >= Math.max(0, zip.length - 65_557); at--)
    if (zip.readUInt32LE(at) === 0x06054b50) {
      end = at
      break
    }
  if (end < 0) throw new Error('Not a zip archive.')
  const out = new Map()
  let at = zip.readUInt32LE(end + 16)
  for (let index = zip.readUInt16LE(end + 10); index > 0; index--) {
    if (zip.readUInt32LE(at) !== 0x02014b50) throw new Error('Damaged zip directory.')
    const method = zip.readUInt16LE(at + 10)
    const compressed = zip.readUInt32LE(at + 20)
    const size = zip.readUInt32LE(at + 24)
    const nameLength = zip.readUInt16LE(at + 28)
    const skip = nameLength + zip.readUInt16LE(at + 30) + zip.readUInt16LE(at + 32)
    const local = zip.readUInt32LE(at + 42)
    const name = zip.toString('utf8', at + 46, at + 46 + nameLength)
    at += 46 + skip
    if (!wanted.has(name)) continue
    if (zip.readUInt32LE(local) !== 0x04034b50) throw new Error(`Damaged zip entry ${name}.`)
    const start = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28)
    const data = zip.subarray(start, start + compressed)
    const bytes = method === 0 ? data : method === 8 ? inflateRawSync(data) : undefined
    if (!bytes || bytes.length !== size) throw new Error(`Unsupported zip entry ${name}.`)
    out.set(name, bytes)
  }
  for (const name of wanted) if (!out.has(name)) throw new Error(`The archive lacks ${name}.`)
  return out
}

/** Stage the helper built for `profile` with libvosk and the notices. */
export async function stageVoiceRuntime(profile) {
  const pin = voicePin()
  const members = unzipMembers(await archive(pin), Object.keys(pin.files))
  const target = join(root, 'crates', 'kchess-node', 'voice')
  rmSync(target, { recursive: true, force: true })
  mkdirSync(target, { recursive: true })
  const executable = process.platform === 'win32' ? 'kchess-voice.exe' : 'kchess-voice'
  const helper = join(root, 'target', profile, executable)
  if (!existsSync(helper)) throw new Error(`Cargo did not produce ${helper}.`)
  copyFileSync(helper, join(target, executable))
  for (const [member, name] of Object.entries(pin.files)) {
    const path = join(target, name)
    writeFileSync(path, members.get(member))
    // The macOS library is universal; KChess ships only this Mac's architecture.
    if (process.platform === 'darwin') {
      const slice = process.arch === 'arm64' ? 'arm64' : 'x86_64'
      execFileSync('lipo', [path, '-thin', slice, '-output', path])
    }
  }
  for (const name of readdirSync(join(root, 'crates', 'kchess-voice', 'licenses')))
    copyFileSync(join(root, 'crates', 'kchess-voice', 'licenses', name), join(target, name))
  if (process.platform !== 'win32')
    for (const name of [executable, ...Object.values(pin.files)])
      chmodSync(join(target, name), 0o755)
  console.info(`[kchess] Staged the voice runtime in crates/kchess-node/voice (${profile}).`)
}
