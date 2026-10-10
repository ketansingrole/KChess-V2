import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { writeRustLicenses } from './rust-licenses.mjs'

/**
 * Build the Rust rules for both runtimes: the Node module for this host, where `@kchess/native`
 * loads it (crates/kchess-node/kchess-native.<platform>-<arch>.node), and the WebAssembly module
 * the renderer imports (apps/desktop/app/assets/rules/kchess.wasm).
 * `--debug` skips release optimizations for quicker local iteration.
 */
const root = fileURLToPath(new URL('..', import.meta.url))
const debug = process.argv.includes('--debug')
const profile = debug ? 'debug' : 'release'
execFileSync('cargo', ['build', '-p', 'kchess-node', '--locked', ...(debug ? [] : ['--release'])], {
  cwd: root,
  stdio: 'inherit',
})
const library = {
  darwin: 'libkchess_node.dylib',
  linux: 'libkchess_node.so',
  win32: 'kchess_node.dll',
}[process.platform]
if (!library) throw new Error(`No native build for ${process.platform}.`)
const source = `${root}target/${profile}/${library}`
if (!existsSync(source)) throw new Error(`Cargo did not produce ${source}.`)
const target = `${root}crates/kchess-node/kchess-native.${process.platform}-${process.arch}.node`
copyFileSync(source, target)
console.info(`[kchess] Built ${target.slice(root.length)} (${profile}).`)

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
// The contract types (core/src/contracts/generated) are rendered from the Rust contracts.
execFileSync(
  'cargo',
  ['run', '-q', '-p', 'kchess-contracts', '--bin', 'export-types', '--locked'],
  { cwd: root, stdio: 'inherit' },
)
writeRustLicenses()
