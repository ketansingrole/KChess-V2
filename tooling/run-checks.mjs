import { spawn } from 'node:child_process'
import { mkdir, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'

/**
 * How to invoke pnpm. `pnpm/setup@v1` installs a standalone executable, so
 * `npm_execpath` is a native binary that node cannot load; corepack instead
 * points it at a `.mjs` launcher that must run under node. A direct `node`
 * invocation (no package manager) leaves it unset.
 */
function pnpmCommand() {
  const execPath = process.env.npm_execpath
  if (execPath && /\.([cm]?js)$/.test(execPath))
    return { cmd: process.execPath, prefix: [execPath] }
  if (execPath) return { cmd: execPath, prefix: [] }
  return { cmd: 'pnpm', prefix: [] }
}

export async function runChecks(steps, mode = 'full') {
  const report = { mode, startedAt: new Date().toISOString(), steps: [] }
  const start = performance.now()
  let exitCode = 0
  try {
    for (const [name, args] of steps) {
      console.info('[kchess] Checking ' + name + '…')
      const stepStart = performance.now()
      const code = await new Promise((resolve, reject) => {
        const { cmd, prefix } = pnpmCommand()
        const child = spawn(cmd, [...prefix, ...args], {
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
      report.steps.push({
        name,
        milliseconds: Math.round(performance.now() - stepStart),
        exitCode: code,
      })
      if (code) {
        exitCode = code
        break
      }
    }
  } catch (cause) {
    console.error(cause)
    report.error = String(cause)
    exitCode = 1
  } finally {
    report.milliseconds = Math.round(performance.now() - start)
    report.exitCode = exitCode
    await mkdir('test-results/guardrails', { recursive: true })
    await writeFile(
      'test-results/guardrails/check-' + mode + '.json',
      JSON.stringify(report, null, 2) + '\n',
    )
  }
  return exitCode
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.exitCode = await runChecks([
    ['format', ['run', 'format:check']],
    ['lint', ['run', 'lint']],
    ['notices', ['run', 'check:notices']],
    ['types', ['run', 'typecheck']],
    ['rust', ['run', 'check:rust']],
    ['native', ['run', 'build:native']],
    ['smoke', ['run', 'test:smoke']],
    ['core', ['run', 'test:core']],
    ['unit', ['run', 'test:unit']],
    ['performance', ['run', 'check:performance']],
  ])
}
