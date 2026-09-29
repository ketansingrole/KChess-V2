// Launches a command with the macOS dev-time Electron binary branded as KChess,
// so the menu bar, Dock, Cmd-Tab and Mission Control show "KChess" and the KChess
// icon instead of "Electron". No-op on other platforms.
//
// macOS labels an app in the app switcher by its bundle folder and executable name,
// not just the name in Info.plist, and shows the icon file inside the bundle. So the
// branded copy is renamed the way electron-builder renames a packaged app: the bundle
// (KChess.app), the main executable, the four helper apps and their executables, plus
// the icon. The copy lives in .dev/ (gitignored) and is rebuilt whenever the installed
// Electron version, this script, or the icon changes.
//
// Usage: node scripts/with-branded-electron.mjs <command> [args...]
import { spawn, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const APP_NAME = 'KChess'
const BUNDLE_ID = 'com.kchess.electron.dev'
const HELPERS = ['Helper', 'Helper (GPU)', 'Helper (Renderer)', 'Helper (Plugin)']
const BUDDY = '/usr/libexec/PlistBuddy'

function plistSet(plist, entries) {
  for (const [key, value] of entries) {
    // `Set` fails when a key is absent, so fall back to `Add`.
    let result = spawnSync(BUDDY, ['-c', `Set :${key} ${value}`, plist])
    if (result.status !== 0) result = spawnSync(BUDDY, ['-c', `Add :${key} string ${value}`, plist])
    if (result.status !== 0)
      throw new Error(`PlistBuddy failed for ${key}: ${result.stderr?.toString() ?? 'unknown'}`)
  }
}

/** Changes whenever the branded copy has to be rebuilt. */
function brandStamp(electronVersion, iconFile) {
  const hash = createHash('sha256')
  hash.update(electronVersion)
  hash.update(fs.readFileSync(fileURLToPath(import.meta.url)))
  if (fs.existsSync(iconFile)) hash.update(fs.readFileSync(iconFile))
  return hash.digest('hex')
}

function brandElectronApp() {
  const electronDist = path.join(root, 'node_modules', 'electron', 'dist')
  const srcBundle = path.join(electronDist, 'Electron.app')
  const devDir = process.env.KCHESS_DEV_DIR ?? path.join(root, '.dev')
  const destBundle = path.join(devDir, `${APP_NAME}.app`)
  const stampFile = path.join(devDir, 'brand-stamp')
  const iconFile = path.join(root, 'build', 'icon.icns')
  const electronVersion = fs.readFileSync(path.join(electronDist, 'version'), 'utf8').trim()
  const stamp = brandStamp(electronVersion, iconFile)
  const binary = path.join(destBundle, 'Contents', 'MacOS', APP_NAME)
  if (
    fs.existsSync(binary) &&
    fs.existsSync(stampFile) &&
    fs.readFileSync(stampFile, 'utf8').trim() === stamp
  ) {
    return destBundle
  }
  console.log(`[kchess] branding dev Electron as ${APP_NAME} (once per Electron/icon change)…`)
  fs.rmSync(destBundle, { recursive: true, force: true })
  fs.rmSync(path.join(devDir, 'Electron.app'), { recursive: true, force: true }) // pre-rename layout
  fs.mkdirSync(devDir, { recursive: true })
  // APFS clone is instant and copy-on-write; fall back to a full copy.
  const clone = spawnSync('cp', ['-Rc', srcBundle, destBundle])
  if (clone.status !== 0) fs.cpSync(srcBundle, destBundle, { recursive: true })

  const contents = path.join(destBundle, 'Contents')
  // Main executable.
  fs.renameSync(path.join(contents, 'MacOS', 'Electron'), binary)
  plistSet(path.join(contents, 'Info.plist'), [
    ['CFBundleName', APP_NAME],
    ['CFBundleDisplayName', APP_NAME],
    ['CFBundleExecutable', APP_NAME],
    ['CFBundleIdentifier', BUNDLE_ID],
    ['CFBundleIconFile', 'icon.icns'],
  ])
  // Helper apps: Electron looks them up as "<executable name> Helper…".
  const frameworks = path.join(contents, 'Frameworks')
  for (const suffix of HELPERS) {
    const oldName = `Electron ${suffix}`
    const newName = `${APP_NAME} ${suffix}`
    const helper = path.join(frameworks, `${newName}.app`)
    fs.renameSync(path.join(frameworks, `${oldName}.app`), helper)
    fs.renameSync(
      path.join(helper, 'Contents', 'MacOS', oldName),
      path.join(helper, 'Contents', 'MacOS', newName),
    )
    const kind = suffix.replace(/^Helper\s*\(?|\)$/g, '').toLowerCase()
    plistSet(path.join(helper, 'Contents', 'Info.plist'), [
      ['CFBundleName', newName],
      ['CFBundleDisplayName', newName],
      ['CFBundleExecutable', newName],
      ['CFBundleIdentifier', kind ? `${BUNDLE_ID}.helper.${kind}` : `${BUNDLE_ID}.helper`],
    ])
  }
  // The icon macOS shows in Cmd-Tab and Mission Control comes from the bundle, not the Dock API.
  const resources = path.join(contents, 'Resources')
  if (fs.existsSync(iconFile)) fs.copyFileSync(iconFile, path.join(resources, 'icon.icns'))
  else console.warn('[kchess] build/icon.icns is missing; keeping the Electron icon')

  // The shipped binary is ad-hoc signed, so re-sign after the edits to keep the seal valid.
  const sign = spawnSync('codesign', ['--force', '--deep', '--sign', '-', destBundle])
  if (sign.status !== 0) {
    console.warn('[kchess] ad-hoc codesign failed, removing signature instead')
    spawnSync('codesign', ['--remove-signature', destBundle])
  }
  // Best effort: make Launch Services forget the name and icon it cached for this path.
  const lsregister =
    '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'
  if (fs.existsSync(lsregister)) spawnSync(lsregister, ['-f', destBundle])
  spawnSync('touch', [destBundle])
  fs.writeFileSync(stampFile, `${stamp}\n`)
  return destBundle
}

const [, , ...args] = process.argv
if (args.length === 0) {
  console.error('usage: node scripts/with-branded-electron.mjs <command> [args...]')
  process.exit(2)
}

let command = args[0]
let commandArgs = args.slice(1)
if (process.platform === 'darwin') {
  try {
    const bundle = brandElectronApp()
    const binary = path.join(bundle, 'Contents', 'MacOS', APP_NAME)
    // electron-vite honors ELECTRON_EXEC_PATH. The plain `electron` CLI resolves the stock
    // executable name, which no longer exists in the branded copy, so run the copy directly.
    process.env.ELECTRON_EXEC_PATH = binary
    if (command === 'electron') command = binary
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

const child = spawn(command, commandArgs, {
  stdio: 'inherit',
  shell: process.platform === 'win32',
})
child.on('exit', (code, signal) => {
  if (signal) process.kill(process.pid, signal)
  else process.exit(code ?? 0)
})
