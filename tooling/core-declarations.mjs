import { readdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
const target = process.argv[2]
if (target !== 'core' && target !== 'hosts/node') throw new Error('Expected core or hosts/node.')
const root = fileURLToPath(new URL('../' + target + '/dist/', import.meta.url))
const entries =
  target === 'hosts/node'
    ? { node: 'hosts/node/src/index' }
    : {
        index: 'core/src/index',
        logger: 'core/src/services/logger',
        gameSession: 'core/src/domain/gameSession',
        gameArchive: 'core/src/domain/gameArchive',
        onlineGame: 'core/src/domain/onlineGame',
        puzzleSession: 'core/src/domain/puzzleSession',
      }
// Source uses bundler resolution. Make the emitted declarations usable by NodeNext consumers.
const files = (await readdir(join(root, 'types'), { recursive: true })).filter((file) =>
  file.endsWith('.d.ts'),
)
const available = new Set(files.map((file) => join(root, 'types', file)))
await Promise.all(
  files.map(async (file) => {
    const path = join(root, 'types', file)
    const source = await readFile(path, 'utf8')
    const result = source.replace(/(['"])(\.{1,2}\/[^'"]+)\1/g, (match, quote, specifier) => {
      const normalized = specifier.replace(/\.ts$/, '')
      const candidate = join(dirname(path), normalized)
      const target = available.has(`${candidate}.d.ts`)
        ? `${normalized}.js`
        : available.has(join(candidate, 'index.d.ts'))
          ? `${normalized}/index.js`
          : undefined
      return target ? `${quote}${target}${quote}` : match
    })
    if (source !== result) await writeFile(path, result)
  }),
)
await Promise.all(
  Object.entries(entries).map(([name, source]) =>
    writeFile(join(root, `${name}.d.ts`), `export * from './types/${source}.js'\n`),
  ),
)
