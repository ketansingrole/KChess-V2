import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, statSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { writeRustLicenses } from './rust-licenses.mjs'
import { replaceNative } from './replace-native.mjs'
import { stageVoiceRuntime } from './voice-runtime.mjs'

/**
 * Build the Rust rules for both runtimes: the Node module for this host, where `@kchess/native`
 * loads it (crates/kchess-node/kchess-native.<platform>-<arch>.node), and the WebAssembly module
 * the renderer imports (apps/desktop/app/assets/rules/kchess.wasm). Also builds the speech
 * recognition helper and stages it with libvosk in crates/kchess-node/voice (voice-runtime.mjs), and
 * the shared core service in crates/kchess-node/service (docs/shared-core.md).
 * `--debug` skips release optimizations for quicker local iteration.
 *
 * Windows size note: MSVC's DLL carries more code/unwind data than the other
 * hosts, so the shipped Windows core passes `--config profile.release.opt-level="z"`
 * below (retaining unwinding and N-API exports). This lives here rather than in
 * `Cargo.toml` so the other hosts keep the default release optimizations; any
 * `cargo build -p kchess-node --release` outside this script builds Windows larger.
 */
const root = fileURLToPath(new URL('..', import.meta.url))
const debug = process.argv.includes('--debug')
const profile = debug ? 'debug' : 'release'
execFileSync(
  'cargo',
  [
    'build',
    '-p',
    'kchess-node',
    '-p',
    'kchess-voice',
    '-p',
    'kchess-service',
    '--locked',
    ...(debug ? [] : ['--release']),
    // MSVC's DLL carries more code/unwind data than the other hosts. Optimize
    // the shipped Windows core for size while retaining unwinding and N-API exports.
    ...(!debug && process.platform === 'win32'
      ? ['--config', 'profile.release.opt-level="z"']
      : []),
  ],
  { cwd: root, stdio: 'inherit' },
)
const library = {
  darwin: 'libkchess_node.dylib',
  linux: 'libkchess_node.so',
  win32: 'kchess_node.dll',
}[process.platform]
if (!library) throw new Error(`No native build for ${process.platform}.`)
const source = `${root}target/${profile}/${library}`
if (!existsSync(source)) throw new Error(`Cargo did not produce ${source}.`)
const target = `${root}crates/kchess-node/kchess-native.${process.platform}-${process.arch}.node`
replaceNative(source, target)
console.info(
  `[kchess] Built ${target.slice(root.length)} (${profile}, ${(statSync(target).size / 1024 / 1024).toFixed(2)} MiB).`,
)
await stageVoiceRuntime(profile)

// The shared core service, which the app starts beside its window (docs/shared-core.md).
const exe = process.platform === 'win32' ? '.exe' : ''
const serviceSource = `${root}target/${profile}/kchess-service${exe}`
if (!existsSync(serviceSource)) throw new Error(`Cargo did not produce ${serviceSource}.`)
mkdirSync(`${root}crates/kchess-node/service`, { recursive: true })
const serviceTarget = `${root}crates/kchess-node/service/kchess-service${exe}`
copyFileSync(serviceSource, serviceTarget)
console.info(
  `[kchess] Staged ${serviceTarget.slice(root.length)} (${profile}, ${(statSync(serviceTarget).size / 1024 / 1024).toFixed(2)} MiB).`,
)

const wasmProfile = debug ? 'dev' : 'wasm'
execFileSync(
  'cargo',
  [
    'build',
    '-p',
    'kchess-wasm',
    '--locked',
    '--target',
    'wasm32-unknown-unknown',
    '--profile',
    wasmProfile,
  ],
  { cwd: root, stdio: 'inherit' },
)
const wasm = `${root}target/wasm32-unknown-unknown/${debug ? 'debug' : 'wasm'}/kchess_wasm.wasm`
if (!existsSync(wasm)) throw new Error(`Cargo did not produce ${wasm}.`)
mkdirSync(`${root}apps/desktop/app/assets/rules`, { recursive: true })
copyFileSync(wasm, `${root}apps/desktop/app/assets/rules/kchess.wasm`)
console.info(`[kchess] Built apps/desktop/app/assets/rules/kchess.wasm (${wasmProfile}).`)
// The contract types (crates/kchess-contracts/ts/generated) are rendered from the Rust contracts.
execFileSync(
  'cargo',
  ['run', '-q', '-p', 'kchess-contracts', '--bin', 'export-types', '--locked'],
  { cwd: root, stdio: 'inherit' },
)
// The TypeScript packages that wrap the rules and contracts, built to their `dist/` for Node
// consumers. Contracts first: the rules and the binding import them.
const build = fileURLToPath(new URL('build-package.mjs', import.meta.url))
for (const args of [
  ['crates/kchess-contracts/ts'],
  ['crates/kchess-wasm/js'],
  ['crates/kchess-node', 'js'],
])
  execFileSync(process.execPath, [build, ...args], { cwd: root, stdio: 'inherit' })
writeRustLicenses()
