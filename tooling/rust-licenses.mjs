import { execFileSync } from 'node:child_process'
import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * Statically linked Rust crates carry no license files into the app, unlike node_modules.
 * Collect the license text of every crate compiled into the native rules (for any supported
 * platform) into crates/kchess-node/THIRD_PARTY_LICENSES.txt, which ships beside the module.
 * Proc-macros and build scripts run only at compile time and are left out.
 */
const root = fileURLToPath(new URL('..', import.meta.url))

export function runtimeCrates(metadata, rootName = 'kchess-node') {
  const packages = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]))
  const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]))
  const start = metadata.packages.find((pkg) => pkg.name === rootName)
  if (!start) throw new Error(`${rootName} is not in the Cargo workspace.`)
  const seen = new Set()
  const queue = [start.id]
  while (queue.length) {
    const id = queue.pop()
    if (seen.has(id)) continue
    seen.add(id)
    for (const dep of nodes.get(id)?.deps ?? []) {
      const runtime = dep.dep_kinds.some((kind) => kind.kind === null)
      const pkg = packages.get(dep.pkg)
      const procMacro = pkg?.targets.some((target) => target.kind.includes('proc-macro'))
      if (runtime && !procMacro) queue.push(dep.pkg)
    }
  }
  return [...seen]
    .map((id) => packages.get(id))
    .filter((pkg) => pkg && pkg.source) // workspace crates are KChess's own code
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version))
}

export function licenseText(pkg) {
  const directory = pkg.manifest_path.replace(/[\\/]Cargo\.toml$/, '')
  const files = readdirSync(directory)
    .filter((name) => /^(?:LICEN[CS]E|COPYING|NOTICE)/i.test(name))
    .sort()
  if (!files.length) return standardLicense(pkg)
  return files
    .map((name) => `--- ${name} ---\n${readFileSync(join(directory, name), 'utf8').trim()}\n`)
    .join('\n')
}

const MIT = `Permission is hereby granted, free of charge, to any person obtaining a copy of this
software and associated documentation files (the "Software"), to deal in the Software
without restriction, including without limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.`

/** Crates published without their license file: the standard text for their SPDX license. */
function standardLicense(pkg) {
  if (/GPL-3\.0/.test(pkg.license ?? ''))
    return `--- GPL-3.0 (the same license as KChess) ---\n${readFileSync(join(root, 'LICENSE'), 'utf8').trim()}\n`
  // `MIT OR ...` (r-efi): the MIT option is elected, so its text is the license that applies.
  if (pkg.license === 'MIT' || /^MIT OR /.test(pkg.license ?? '')) {
    const holders = pkg.authors?.length ? pkg.authors.join(', ') : `the ${pkg.name} authors`
    return `--- MIT ---\nCopyright (c) ${holders}\n\n${MIT}\n`
  }
  throw new Error(`${pkg.name} ${pkg.version} ships no license file for ${pkg.license}.`)
}

export function writeRustLicenses() {
  const metadata = JSON.parse(
    execFileSync('cargo', ['metadata', '--format-version', '1', '--locked'], {
      cwd: root,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
    }),
  )
  const crates = runtimeCrates(metadata)
  // Identical texts (Apache-2.0 above all) are printed once, with every crate they cover.
  const texts = new Map()
  for (const pkg of crates) {
    const text = licenseText(pkg)
    texts.set(text, [...(texts.get(text) ?? []), `${pkg.name} ${pkg.version}`])
  }
  const index = crates
    .map((pkg) => `- ${pkg.name} ${pkg.version} (${pkg.license}) ${pkg.repository ?? ''}`.trim())
    .join('\n')
  const sections = [...texts].map(
    ([text, names]) =>
      `${'='.repeat(78)}\nApplies to: ${names.join(', ')}\n${'='.repeat(78)}\n\n${text}`,
  )
  writeFileSync(
    join(root, 'crates/kchess-node/THIRD_PARTY_LICENSES.txt'),
    `Rust crates compiled into the KChess rules: kchess-native.*.node, and the renderer's kchess.wasm (a subset):\n\n${index}\n\n${sections.join('\n')}`,
  )
  console.info(`[kchess] Collected licenses for ${crates.length} Rust crates.`)
}

if (process.argv[1] === fileURLToPath(import.meta.url)) writeRustLicenses()
