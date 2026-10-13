import { spawn, execFileSync } from 'node:child_process'
import {
  mkdirSync,
  openSync,
  closeSync,
  readFileSync,
  statSync,
  writeFileSync,
  unlinkSync,
} from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { setTimeout as delay } from 'node:timers/promises'
import { attempt, attemptAsync } from './capture-error.mjs'

const root = fileURLToPath(new URL('..', import.meta.url))
const url = 'http://127.0.0.1:3000'
const lockPath = join(root, '.dev', 'session.json')

function processes() {
  if (process.platform === 'win32') {
    return JSON.parse(
      execFileSync(
        'powershell.exe',
        [
          '-NoProfile',
          '-Command',
          'Get-CimInstance Win32_Process | Select-Object ProcessId,CommandLine | ConvertTo-Json',
        ],
        { encoding: 'utf8' },
      ),
    ).map((row) => ({ pid: row.ProcessId, command: row.CommandLine ?? '' }))
  }
  return execFileSync('ps', ['-axo', 'pid=,command='], { encoding: 'utf8' })
    .trim()
    .split('\n')
    .map((line) => {
      const match = line.trim().match(/^(\d+)\s+(.*)$/)
      return { pid: Number(match?.[1]), command: match?.[2] ?? '' }
    })
}
function alive(pid) {
  // The thrown error is the signal (no such process): `attempt` records it as a value.
  return !('error' in attempt(() => process.kill(pid, 0)))
}
function stop(pid, group = false) {
  // Stopping an already-dead process is expected during stale-session recovery.
  void attempt(() => {
    if (process.platform === 'win32') execFileSync('taskkill', ['/PID', String(pid), '/T', '/F'])
    else process.kill(group ? -pid : pid, 'SIGTERM')
  })
}
async function nuxtUp() {
  // A refused connection is the expected "not ready yet" signal while polling.
  const probed = await attemptAsync(async () => {
    const response = await fetch(url, { signal: AbortSignal.timeout(1500) })
    return response.ok
  })
  return 'value' in probed ? probed.value : false
}

const projectProcesses = processes().filter(
  (row) => row.command.includes(root) && /(?:nuxt|nuxi|electron-vite).*\bdev\b/.test(row.command),
)
const rendererUp = await nuxtUp()
const electronUp = projectProcesses.some((row) => /electron-vite.*\bdev\b/.test(row.command))
if (rendererUp && electronUp) {
  console.info(`[kchess] Reusing the running dev session at ${url}.`)
  process.exit(0)
}

mkdirSync(dirname(lockPath), { recursive: true })
try {
  const fd = openSync(lockPath, 'wx')
  writeFileSync(fd, JSON.stringify({ pid: process.pid }))
  closeSync(fd)
} catch (error) {
  if (error.code !== 'EEXIST') throw error
  // Another launcher may have just created the file and not written its PID yet.
  if (Date.now() - statSync(lockPath).mtimeMs < 5000) {
    console.info('[kchess] A dev session is already starting. Try again shortly.')
    process.exit(0)
  }
  const parsed = attempt(() => JSON.parse(readFileSync(lockPath, 'utf8')))
  // An unreadable lock file is treated as stale and recreated below.
  const session = 'value' in parsed ? parsed.value : undefined
  if (session && alive(session.pid)) {
    console.info('[kchess] A dev session is already starting or stopping. Try again shortly.')
    process.exit(0)
  }
  unlinkSync(lockPath)
  const fd = openSync(lockPath, 'wx')
  writeFileSync(fd, JSON.stringify({ pid: process.pid }))
  closeSync(fd)
}

const children = []
let stopping = false
function cleanup(code = 0) {
  if (stopping) return
  stopping = true
  for (const child of children) if (child.pid) stop(child.pid, true)
  // The lock may already be gone when two launchers race; the value is discarded.
  void attempt(() => unlinkSync(lockPath))
  process.exitCode = code
}
process.once('SIGINT', () => cleanup())
process.once('SIGTERM', () => cleanup())
process.once('exit', () => cleanup(process.exitCode ?? 0))

try {
  // Recover only processes whose command belongs to this checkout.
  for (const row of projectProcesses) stop(row.pid)
  for (let poll = 0; poll < 20 && (await nuxtUp()); poll++) await delay(250)
  if (await nuxtUp())
    throw new Error(`Port 3000 is occupied by another server. Stop it before starting KChess.`)
  function launch(args, env = process.env) {
    const child = spawn(process.execPath, args, {
      cwd: root,
      env,
      stdio: 'inherit',
      detached: process.platform !== 'win32',
    })
    children.push(child)
    child.once('error', (error) => {
      console.error(error)
      cleanup(1)
    })
    child.once('exit', (code) => {
      if (!stopping) cleanup(code ?? 1)
    })
    return child
  }
  // The core runs the Rust rules when they are built for this host (incremental, seconds).
  // Fail fast: continuing with a missing native module leaves the core unable to
  // start and surfaces only as an obscure `nativeRules() null` later.
  try {
    execFileSync(process.execPath, [join(root, 'tooling/build-native.mjs')], {
      cwd: root,
      stdio: 'inherit',
    })
  } catch (error) {
    throw new Error(
      `Native rules did not build; refusing to launch without them. ${error.message}`,
      { cause: error },
    )
  }
  launch([
    join(root, 'node_modules/nuxt/bin/nuxt.mjs'),
    'dev',
    'apps/desktop',
    '--host',
    '127.0.0.1',
    '--port',
    '3000',
  ])
  let ready = false
  for (let poll = 0; poll < 120 && !stopping; poll++) {
    if (await nuxtUp()) {
      ready = true
      break
    }
    await delay(500)
  }
  if (!stopping && !ready) throw new Error('Nuxt did not become ready within 60 seconds.')
  if (!stopping)
    launch([join(root, 'tooling/with-branded-electron.mjs'), 'electron-vite', 'dev', '--watch'], {
      ...process.env,
      KCHESS_NUXT_URL: url,
    })
} catch (error) {
  console.error('[kchess]', error.message)
  cleanup(1)
}
