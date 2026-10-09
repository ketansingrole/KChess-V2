import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { writeRustLicenses } from './rust-licenses.mjs'

/**
 * Build the Rust rules (`crates/kchess-node`) for this host and place the module where
 * `@kchess/native` loads it: crates/kchess-node/kchess-native.<platform>-<arch>.node.
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
writeRustLicenses()
