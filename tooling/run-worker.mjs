import { spawn } from 'node:child_process'

/**
 * Runs a benchmark worker (`node <args>`) and returns the JSON object it printed last, with its
 * stderr. Fails with both outputs when the worker exits badly or prints no JSON.
 */
export async function runJsonWorker(args, { cwd, env = process.env, label }) {
  const child = spawn(process.execPath, args, { cwd, env, stdio: ['ignore', 'pipe', 'pipe'] })
  let stdout = ''
  let stderr = ''
  child.stdout.setEncoding('utf8').on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.setEncoding('utf8').on('data', (chunk) => {
    stderr += chunk
  })
  const status = await new Promise((resolvePromise, reject) => {
    child.once('error', reject)
    child.once('close', (code, signal) => resolvePromise({ code, signal }))
  })
  if (status.code !== 0)
    throw new Error(`${label} failed: ${status.signal ?? status.code}\n${stderr}\n${stdout}`)
  const line = stdout
    .trim()
    .split('\n')
    .reverse()
    .find((row) => row.trim().startsWith('{'))
  if (!line) throw new Error(`Missing JSON from ${label}:\n${stderr}\n${stdout}`)
  try {
    return { ...JSON.parse(line), stderr: stderr.trim() || undefined }
  } catch (error) {
    throw new Error(`Could not parse ${label} output: ${error}\n${stderr}\n${stdout}`, {
      cause: error,
    })
  }
}
