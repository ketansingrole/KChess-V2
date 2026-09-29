// Launches a command with the macOS dev-time Electron binary branded as KChess,
// so the menu bar, Dock, and Cmd-Tab show "KChess" instead of "Electron".
// No-op on other platforms.
//
// The branded copy lives in .dev/ (gitignored) and is only rebuilt when the
// installed Electron version changes. Branding edits just the top-level
// Info.plist (CFBundleName/DisplayName/Identifier); the executable keeps its
// stock name so the `electron` package can still resolve it.
//
// Usage: node scripts/with-branded-electron.mjs <command> [args...]
import { spawn, spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

function brandElectronApp() {
  const electronDist = path.join(root, 'node_modules', 'electron', 'dist')
  const srcBundle = path.join(electronDist, 'Electron.app')
  const devDir = path.join(root, '.dev')
  const destBundle = path.join(devDir, 'Electron.app')
  const stampFile = path.join(devDir, 'electron-version')
  const electronVersion = fs.readFileSync(path.join(electronDist, 'version'), 'utf8').trim()
  if (
    fs.existsSync(destBundle) &&
    fs.existsSync(stampFile) &&
    fs.readFileSync(stampFile, 'utf8').trim() === electronVersion
  ) {
    return destBundle
  }
  console.log('[kchess] branding dev Electron.app as KChess (one-time per Electron version)…')
  fs.rmSync(destBundle, { recursive: true, force: true })
  fs.mkdirSync(devDir, { recursive: true })
  // APFS clone is instant and copy-on-write; fall back to a full copy.
  const clone = spawnSync('cp', ['-Rc', srcBundle, destBundle])
  if (clone.status !== 0) fs.cpSync(srcBundle, destBundle, { recursive: true })
  const plist = path.join(destBundle, 'Contents', 'Info.plist')
  const buddy = '/usr/libexec/PlistBuddy'
  for (const [key, value] of [
    [':CFBundleName', 'KChess'],
    [':CFBundleDisplayName', 'KChess'],
    [':CFBundleIdentifier', 'com.kchess.electron.dev'],
  ]) {
    const result = spawnSync(buddy, ['-c', `Set ${key} ${value}`, plist])
    if (result.status !== 0) {
      throw new Error(
        `PlistBuddy failed for ${key}: ${result.stderr?.toString() ?? 'unknown error'}`,
      )
    }
  }
  // The shipped binary is ad-hoc signed, so re-apply an equivalent signature
  // after editing the plist to keep the bundle seal consistent.
  const sign = spawnSync('codesign', ['--force', '--deep', '--sign', '-', destBundle])
  if (sign.status !== 0) {
    console.warn('[kchess] ad-hoc codesign failed, removing signature instead')
    spawnSync('codesign', ['--remove-signature', destBundle])
  }
  fs.writeFileSync(stampFile, `${electronVersion}\n`)
  return destBundle
}

const [, , ...args] = process.argv
if (args.length === 0) {
  console.error('usage: node scripts/with-branded-electron.mjs <command> [args...]')
  process.exit(2)
}

if (process.platform === 'darwin') {
  try {
    const bundle = brandElectronApp()
    // `electron` package honors ELECTRON_OVERRIDE_DIST_PATH;
    // electron-vite honors ELECTRON_EXEC_PATH.
    process.env.ELECTRON_OVERRIDE_DIST_PATH = path.join(root, '.dev')
    process.env.ELECTRON_EXEC_PATH = path.join(bundle, 'Contents', 'MacOS', 'Electron')
  } catch (error) {
    console.warn(
      '[kchess] dev branding failed, falling back to stock Electron:',
      error instanceof Error ? error.message : error,
    )
  }
}

// PATH lookup works without a shell on POSIX (execvp); Windows needs a shell
// to resolve .cmd shims. (Avoid shell:true on POSIX: node concatenates args
// without escaping, which would split quoted groups like concurrently's.)
// Ensure project-local binaries (electron, concurrently, electron-vite, …)
// resolve even when invoked outside of `npm run` (which sets this up itself).
const localBin = path.join(root, 'node_modules', '.bin')
if (!process.env.PATH?.split(path.delimiter).includes(localBin)) {
  process.env.PATH = `${localBin}${path.delimiter}${process.env.PATH ?? ''}`
}

const child = spawn(args[0], args.slice(1), {
  stdio: 'inherit',
  shell: process.platform === 'win32',
})
child.on('exit', (code, signal) => {
  if (signal) process.kill(process.pid, signal)
  else process.exit(code ?? 0)
})
