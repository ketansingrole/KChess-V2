import { spawn } from 'node:child_process'
import { lstat, mkdir, readlink, unlink } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { verifyRenderer } from './verify-package.mjs'

const root = fileURLToPath(new URL('..', import.meta.url))
const desktop = join(root, 'apps/desktop')
// Nuxt generate creates a dist -> .output/public alias when dist doesn't exist. electron-builder
// also writes to dist, which would put installers inside the renderer and bundle them recursively.
const installerOutput = join(root, 'dist')
try {
  if ((await lstat(installerOutput)).isSymbolicLink()) {
    if (resolve(root, await readlink(installerOutput)) !== join(desktop, '.output', 'public'))
      throw new Error('dist must be a directory, not a symlink to another location.')
    await unlink(installerOutput)
  }
} catch (cause) {
  if (cause.code !== 'ENOENT') throw cause
}
await mkdir(installerOutput, { recursive: true })

async function run(cli, args, env = process.env, cwd = root) {
  const code = await new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [fileURLToPath(new URL(cli, import.meta.url)), ...args], {
      cwd,
      env,
      stdio: 'inherit',
    })
    child.once('error', reject)
    child.once('exit', (code) => resolve(code ?? 1))
    for (const signal of ['SIGINT', 'SIGTERM']) {
      const stop = () => child.kill(signal)
      process.once(signal, stop)
      child.once('exit', () => process.removeListener(signal, stop))
    }
  })
  if (code) process.exit(code)
}
// The core loads the Rust rules from @kchess/native; package them for this host.
await run('build-native.mjs', [])
await run('../node_modules/nuxt/bin/nuxt.mjs', ['typecheck', 'apps/desktop'])
await run('../node_modules/typescript/bin/tsc', [
  '--noEmit',
  '-p',
  'apps/desktop/tsconfig.electron.json',
])
await run('../node_modules/vue-tsc/bin/vue-tsc.js', ['--noEmit', '-p', 'tsconfig.tests.json'])
await run('../node_modules/nuxt/bin/nuxt.mjs', ['generate', 'apps/desktop'], {
  ...process.env,
  NUXT_APP_BASE_URL: './',
})
await verifyRenderer(join(desktop, '.output', 'public'))
await run('../node_modules/electron-vite/bin/electron-vite.js', ['build'], process.env, desktop)
