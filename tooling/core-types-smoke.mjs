import { spawnSync } from 'node:child_process'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('..', import.meta.url))
const directory = await mkdtemp(join(tmpdir(), 'kchess-types-'))
try {
  const core = JSON.stringify(join(root, 'crates/kchess-node/dist/index.js'))
  const node = JSON.stringify(join(root, 'hosts/node/dist/index.js'))
  await writeFile(
    join(directory, 'consumer.mts'),
    `import { createKChessCore, type CorePlatform } from ${core}
import { createNodeCore, type NodeCoreOptions } from ${node}
declare const platform: CorePlatform
declare const options: NodeCoreOptions
const direct = createKChessCore(platform)
const isolated = await createNodeCore(options)
await direct.loadData()
await isolated.loadData()
await isolated.close()
`,
  )
  await writeFile(
    join(directory, 'tsconfig.json'),
    JSON.stringify({
      compilerOptions: {
        target: 'ES2023',
        module: 'NodeNext',
        moduleResolution: 'NodeNext',
        strict: true,
        noEmit: true,
        types: ['node'],
        typeRoots: [join(root, 'node_modules/@types')],
      },
      include: ['consumer.mts'],
    }),
  )
  const result = spawnSync(
    process.execPath,
    [join(root, 'node_modules/typescript/bin/tsc'), '-p', join(directory, 'tsconfig.json')],
    { stdio: 'inherit' },
  )
  if (result.error) throw result.error
  process.exitCode = result.status ?? 1
  if (!process.exitCode)
    console.info('ok   core and Node host declarations work in NodeNext consumers')
} finally {
  await rm(directory, { recursive: true, force: true })
}
