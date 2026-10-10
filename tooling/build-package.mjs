import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync, readdirSync, rmSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'vite'
import { rewriteDeclarationSpecifiers } from './declarations.mjs'
import { isPackageExternal, isPackageSource } from './package-paths.mjs'

/**
 * Build one workspace package to `dist/`: each TypeScript module of its source directory becomes
 * one JavaScript file at the same relative path (so `exports` can map `./*` to `./dist/*`), with
 * declarations beside it. Other workspace packages and third-party dependencies stay external:
 * they resolve at run time through their own packages.
 *
 * Usage: node tooling/build-package.mjs <package directory> [source subdirectory]
 */
const root = fileURLToPath(new URL('..', import.meta.url))
const [packageArg, sourceArg = '.'] = process.argv.slice(2)
if (!packageArg)
  throw new Error('Usage: node tooling/build-package.mjs <package directory> [source]')
const packageDir = resolve(root, packageArg)
const sourceDir = resolve(packageDir, sourceArg)
const outDir = join(packageDir, 'dist')

const inputs = readdirSync(sourceDir, { recursive: true })
  .filter(isPackageSource)
  .map((file) => join(sourceDir, file))

rmSync(outDir, { recursive: true, force: true })
await build({
  root: packageDir,
  configFile: false,
  logLevel: 'warn',
  build: {
    ssr: true,
    target: 'node24',
    outDir,
    emptyOutDir: true,
    rollupOptions: {
      input: inputs,
      external: isPackageExternal,
      output: {
        format: 'es',
        preserveModules: true,
        preserveModulesRoot: sourceDir,
        entryFileNames: '[name].js',
      },
    },
  },
})
// The three packages import one another's sources, so their declarations come from one program
// rooted at `crates/` (tsconfig.declarations.json). Each package takes its own subtree.
execFileSync(
  process.execPath,
  [join(root, 'node_modules/typescript/bin/tsc'), '-p', join(root, 'tsconfig.declarations.json')],
  { cwd: root, stdio: 'inherit' },
)
const declarations = join(
  root,
  'node_modules/.cache/kchess-declarations',
  relative(join(root, 'crates'), sourceDir),
)
for (const file of readdirSync(declarations, { recursive: true })) {
  if (!file.endsWith('.d.ts')) continue
  mkdirSync(dirname(join(outDir, file)), { recursive: true })
  copyFileSync(join(declarations, file), join(outDir, file))
}
await rewriteDeclarationSpecifiers(outDir)
console.info(`[kchess] Built ${packageDir.slice(root.length)}/dist.`)
