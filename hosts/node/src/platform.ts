import { spawn } from 'node:child_process'
import { mkdir } from 'node:fs/promises'
import { homedir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { createRequire } from 'node:module'
import type { CorePlatform } from '@kchess/core'
import { systemSecrets } from './secrets'

export interface NodeCoreOptions {
  dataDir?: string
  bundledEnginePath?: string
  managedEngineDir?: string
  puzzleWorkerPath?: string
  /** Disable credentials for unattended tools and isolated tests. */
  disableCredentials?: boolean
  /** Print login URLs instead of launching a browser on an SSH/headless host. */
  browser?: 'open' | 'print'
}
export async function nodePlatform(options: NodeCoreOptions): Promise<CorePlatform> {
  const dataDir = resolve(options.dataDir ?? join(homedir(), '.kchess', 'node'))
  await mkdir(dataDir, { recursive: true, mode: 0o700 })
  const require = createRequire(import.meta.url)
  const bundledEnginePath =
    options.bundledEnginePath ??
    join(dirname(require.resolve('stockfish/package.json')), 'bin', 'stockfish-19-lite.js')
  return {
    dataDir,
    bundledEnginePath,
    managedEngineDir: options.managedEngineDir ?? join(dataDir, 'engines'),
    puzzleWorkerPath: options.puzzleWorkerPath,
    secrets: systemSecrets(dataDir, options.disableCredentials),
    onBattery: () => false,
    async openExternal(url) {
      const parsed = new URL(url)
      if (parsed.protocol !== 'https:') throw new Error('Only HTTPS browser URLs are supported.')
      if (options.browser === 'print') {
        process.stderr.write(`Sign in: ${url}\n`)
        return
      }
      const executable =
        process.platform === 'darwin'
          ? 'open'
          : process.platform === 'win32'
            ? 'rundll32.exe'
            : 'xdg-open'
      const args = process.platform === 'win32' ? ['url.dll,FileProtocolHandler', url] : [url]
      await new Promise<void>((resolve, reject) => {
        const child = spawn(executable, args, { stdio: 'ignore', windowsHide: true })
        child.once('error', reject)
        child.once('exit', (code) =>
          code === 0
            ? resolve()
            : reject(new Error('Could not open the browser. Use --browser print.')),
        )
      })
    },
  }
}
