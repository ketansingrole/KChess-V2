import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { prepareVoiceModel } from './prepare-voice-model.mjs'

const root = fileURLToPath(new URL('..', import.meta.url))
async function run(cli, args, env = process.env) {
  const code = await new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [fileURLToPath(new URL(cli, import.meta.url)), ...args], {
      cwd: root,
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
await prepareVoiceModel()
await run('../node_modules/nuxt/bin/nuxt.mjs', ['typecheck'])
await run('../node_modules/typescript/bin/tsc', ['--noEmit', '-p', 'tsconfig.electron.json'])
await run('../node_modules/vue-tsc/bin/vue-tsc.js', ['--noEmit', '-p', 'tsconfig.tests.json'])
await run('../node_modules/nuxt/bin/nuxt.mjs', ['generate'], {
  ...process.env,
  NUXT_APP_BASE_URL: './',
})
await run('../node_modules/electron-vite/bin/electron-vite.js', ['build'])
