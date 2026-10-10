import { readdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * Source uses bundler resolution (extensionless relative imports). Rewrite the relative
 * specifiers in emitted declarations so NodeNext consumers can load them: `./x` becomes `./x.js`,
 * and `./dir` becomes `./dir/index.js`.
 */
export async function rewriteDeclarationSpecifiers(distDir) {
  const files = (await readdir(distDir, { recursive: true }))
    .filter((file) => file.endsWith('.d.ts'))
    .map((file) => join(distDir, file))
  const available = new Set(files)
  await Promise.all(
    files.map(async (path) => {
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
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const target = process.argv[2]
  if (!target) throw new Error('Usage: node tooling/declarations.mjs <dist directory>')
  await rewriteDeclarationSpecifiers(resolve(target))
}
