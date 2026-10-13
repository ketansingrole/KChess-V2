// Runs a command in CI's Linux environment on this machine (Docker; on macOS, e.g. Colima).
// Usage: pnpm run linux [--amd64] [--rebuild] [--] [command…]   (default: pnpm run check)
//
// The working tree, including uncommitted and untracked files, is synced into a cached
// container volume, so Linux node_modules, Cargo targets and native modules never touch the
// host checkout. `test-results/` is copied back to `test-results/linux/` after every run.
// `--amd64` matches CI's x86-64 runners through emulation (slower); the default is native.
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('..', import.meta.url))
const args = process.argv.slice(2)
const flag = (name) => {
  const index = args.indexOf(name)
  if (index >= 0) args.splice(index, 1)
  return index >= 0
}
const amd64 = flag('--amd64')
const rebuild = flag('--rebuild')
if (args[0] === '--') args.shift()
const command = args.length ? args : ['pnpm', 'run', 'check']

const docker = (dockerArgs, options = {}) =>
  spawnSync('docker', dockerArgs, { stdio: 'inherit', ...options })
if (docker(['info'], { stdio: 'ignore' }).status !== 0) {
  console.error('[linux] Docker is not running. On macOS start it with `colima start`.')
  process.exit(1)
}

const arch = amd64 ? 'amd64' : process.arch === 'arm64' ? 'arm64' : 'amd64'
const platform = `linux/${arch}`
const versions = {
  NODE_VERSION: readFileSync(join(root, '.nvmrc'), 'utf8').trim(),
  PNPM_VERSION: JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).packageManager.split(
    '@',
  )[1],
  RUST_VERSION: /channel\s*=\s*"([^"]+)"/.exec(
    readFileSync(join(root, 'rust-toolchain.toml'), 'utf8'),
  )[1],
}
const dockerfile = join(root, 'tooling/linux/Dockerfile')
const digest = createHash('sha256')
  .update(readFileSync(dockerfile))
  .update(JSON.stringify(versions))
  .digest('hex')
  .slice(0, 12)
const image = `kchess-linux:${arch}-${digest}`

if (rebuild || docker(['image', 'inspect', image], { stdio: 'ignore' }).status !== 0) {
  console.info(`[linux] Building ${image} (once per toolchain change)…`)
  const build = docker([
    'build',
    '--platform',
    platform,
    ...Object.entries(versions).flatMap(([key, value]) => ['--build-arg', `${key}=${value}`]),
    '-t',
    image,
    '-f',
    dockerfile,
    join(root, 'tooling/linux'),
  ])
  if (build.status !== 0) process.exit(build.status ?? 1)
}

// Build output and dependencies stay in the volume; everything else mirrors the host tree.
const excludes = [
  'node_modules',
  'target',
  'dist',
  'out',
  '.output',
  '.nuxt',
  '.dev',
  '.data',
  'test-results',
  'playwright-report',
  'coverage',
  '.pnpm-store',
  '.eslintcache',
  '*.tsbuildinfo',
  '/crates/kchess-node/*.node',
]
const sync = [
  'rsync -a --delete',
  ...excludes.map((pattern) => `--exclude='${pattern}'`),
  '/src/ /work/',
].join(' ')
const quoted = command.map((part) => `'${part.replaceAll("'", `'\\''`)}'`).join(' ')
const script = [
  'set -e',
  sync,
  'pnpm install --frozen-lockfile',
  // Keep the command's status; copy its results back whether it passed or not.
  `set +e; ${quoted}; status=$?`,
  'rm -rf /results/* && { [ ! -d test-results ] || cp -R test-results/. /results/; }',
  'exit $status',
].join('\n')

const results = join(root, 'test-results/linux')
rmSync(results, { recursive: true, force: true })
mkdirSync(results, { recursive: true })
const volume = (name) => `kchess-linux-${arch}-${name}`
console.info(`[linux] ${platform}: ${command.join(' ')}`)
const run = docker([
  'run',
  '--rm',
  '--init',
  ...(process.stdout.isTTY ? ['-t'] : []),
  '--platform',
  platform,
  '--shm-size=2g',
  // Chromium's sandbox needs the user-namespace syscalls Docker's default profile blocks.
  '--security-opt',
  'seccomp=unconfined',
  '-e',
  'CI=true',
  '-e',
  'npm_config_store_dir=/pnpm-store',
  '-v',
  `${root}:/src:ro`,
  '-v',
  `${volume('work')}:/work`,
  '-v',
  `${volume('pnpm')}:/pnpm-store`,
  '-v',
  `${volume('cargo')}:/usr/local/cargo/registry`,
  '-v',
  `${results}:/results`,
  image,
  'bash',
  '-c',
  script,
])
process.exit(run.status ?? 1)
