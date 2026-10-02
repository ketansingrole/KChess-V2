import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { prepareVoiceModel } from './prepare-voice-model.mjs'

const root = fileURLToPath(new URL('..', import.meta.url))
const packaged = process.argv.includes('--packaged')
const candidates =
  process.platform === 'darwin'
    ? ['dist/mac-arm64/KChess.app/Contents/MacOS/KChess']
    : process.platform === 'win32'
      ? ['dist/win-unpacked/KChess.exe']
      : ['dist/linux-unpacked/kchess', 'dist/linux-unpacked/kchess-electron']
const executable = candidates.map((file) => join(root, file)).find(existsSync)
if (packaged && !executable) throw new Error('Packaged app missing. Run npm run pack first.')
if (!packaged && !existsSync(join(root, 'out/main/index.js')))
  throw new Error('Built app missing. Run npm run build first.')
await prepareVoiceModel()

const env = {
  ...process.env,
  KCHESS_NUXT_URL: '',
  KCHESS_DEV_DIR: join(root, '.dev', 'automation'),
  KCHESS_PACKAGED_EXEC_PATH: packaged ? executable : '',
}
// The wrapper chooses a separate branded copy, preserving an existing user's dev session.
const child = spawn(
  process.execPath,
  [
    join(root, 'scripts/with-branded-electron.mjs'),
    process.execPath,
    join(root, 'node_modules/@playwright/test/cli.js'),
    'test',
    ...(packaged ? ['--grep', '@packaged'] : []),
  ],
  { cwd: root, env, stdio: 'inherit' },
)
child.on('error', (error) => {
  console.error(error)
  process.exitCode = 1
})
child.on('exit', (code) => {
  process.exitCode = code ?? 1
})
for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => child.kill(signal))
