// Compare fresh macOS arm64 production bundles; no release or publication.
import { readdir, stat, readFile, mkdir, writeFile } from 'node:fs/promises'
import { resolve, join, relative } from 'node:path'
import { createHash } from 'node:crypto'
import { getRawHeader } from '@electron/asar'

const roots = process.argv.slice(2).filter((arg) => !arg.startsWith('--'))
const zipDir = process.argv.find((arg) => arg.startsWith('--zip-dir='))?.slice('--zip-dir='.length)
if (roots.length !== 2)
  throw new Error('Usage: size.mjs <typescript-root> <rust-root> [--zip-dir=<directory>]')
async function files(root) {
  const result = []
  async function visit(dir) {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name)
      if (entry.isDirectory()) await visit(path)
      else if (entry.isFile())
        result.push({ path: relative(root, path), bytes: (await stat(path)).size })
    }
  }
  await visit(root)
  return result
}
const total = (rows) => rows.reduce((sum, row) => sum + row.bytes, 0)
const results = []
for (const [index, input] of roots.entries()) {
  const root = resolve(input)
  const app =
    index === 0 && process.env.KCHESS_SIZE_BASELINE_APP
      ? resolve(process.env.KCHESS_SIZE_BASELINE_APP)
      : join(root, 'dist/mac-arm64/KChess.app')
  const archive = join(app, 'Contents/Resources/app.asar')
  const entries = []
  function visitAsar(nodes, prefix = '') {
    for (const [name, value] of Object.entries(nodes)) {
      const path = prefix + name
      if (value.files) visitAsar(value.files, path + '/')
      else if (typeof value.size === 'number')
        entries.push({ path, bytes: value.size, unpacked: !!value.unpacked })
    }
  }
  visitAsar(getRawHeader(archive).header.files)
  const disk = await files(app)
  const renderer = await files(join(root, 'apps/desktop/.output/public'))
  results.push({
    variant: index === 0 ? 'typescript' : 'rust',
    root,
    appPath: app,
    appLogicalBytes: total(disk),
    appFileCount: disk.length,
    appPayloadBytes: total(entries),
    asarBytes: (await stat(archive)).size,
    asarSha256: createHash('sha256')
      .update(await readFile(archive))
      .digest('hex'),
    rendererBytes: total(renderer),
    mainBytes: total(await files(join(root, 'apps/desktop/out/main'))),
    nativeBytes: total(entries.filter((row) => row.path.endsWith('.node'))),
    rulesWasmBytes: total(renderer.filter((row) => row.path.endsWith('.wasm'))),
    stockfishBytes: total(entries.filter((row) => row.path.startsWith('node_modules/stockfish/'))),
    otherDependenciesBytes: total(
      entries.filter(
        (row) =>
          row.path.startsWith('node_modules/') &&
          !row.path.startsWith('node_modules/stockfish/') &&
          !row.path.endsWith('.node'),
      ),
    ),
    archiveNote:
      'Benchmark-only headless core/dist was absent from the TypeScript app when packaged, matching its historical release build. The Rust package excludes binding dist files through its committed configuration.',
    largestPayloadFiles: [...entries].sort((a, b) => b.bytes - a.bytes).slice(0, 15),
    entries,
  })
  if (zipDir) {
    const result = results.at(-1)
    const archivePath = join(zipDir, `kchess-${result.variant}-benchmark.zip`)
    result.benchmarkZipBytes = (await stat(archivePath)).size
    result.benchmarkZipSha256 = createHash('sha256')
      .update(await readFile(archivePath))
      .digest('hex')
    result.benchmarkZipNote =
      'Unsigned ditto -c -k --sequesterRsrc --keepParent archive of verified .app including macOS metadata. Download-size proxy, not a signed/notarized release ZIP or DMG. Verify CRC separately with unzip -t.'
  }
}
await mkdir('docs/benchmarks/raw', { recursive: true })
await writeFile(
  process.env.KCHESS_SIZE_OUTPUT ?? 'docs/benchmarks/raw/size.json',
  JSON.stringify(
    {
      measuredAt: new Date().toISOString(),
      methodology:
        'Fresh unsigned electron-builder macOS arm64 directory builds, publish never, same locally installed Electron distribution. Logical file bytes exclude symlinks, APFS allocation and compression. ASAR payload includes unpacked entries exactly once. Renderer includes all built public files; app payload excludes voice archive according to each revision packaging rules.',
      results,
    },
    null,
    2,
  ) + '\n',
)
console.info(
  JSON.stringify(
    results.map((result) => {
      const summary = { ...result }
      delete summary.entries
      delete summary.largestPayloadFiles
      return summary
    }),
    null,
    2,
  ),
)
