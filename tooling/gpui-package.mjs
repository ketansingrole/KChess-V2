// Packages the GPUI app (apps/gpui) for this machine, with its own native Stockfish and the voice
// runtime (the recognition helper, libvosk and their notices, tooling/voice-runtime.mjs) beside it,
// in the formats the Electron app ships (electron-builder, given the staged app as --prepackaged):
// macOS: a .dmg and a .zip (Apple Silicon); Windows: the per-user NSIS installer (.exe); Linux: an
// .AppImage and a .deb. The updater installs each kind (apps/gpui/src/updates.rs). Every file gets
// a line in SHA256SUMS.txt. Output: apps/gpui/dist/.
//
// Usage: node tooling/gpui-package.mjs [--skip-build] [--final]
//   --skip-build  package the release binary already built
//   --final       the identity that replaces the Electron app (com.kchess.app, "KChess", its
//                 installer GUID and .deb name), with the update manifests (latest*.yml) Electron's
//                 updater reads; without it the preview installs beside Electron as "KChess Preview"
//                 and writes no manifests, so Electron never mistakes it for its own update
//
// Stockfish is the latest official release, fetched and verified exactly as the app's own
// download is (crates/kchess-core/examples/fetch_stockfish.rs) and cached under target/.
//
// macOS signing: without KCHESS_SIGN_IDENTITY the bundle is signed ad hoc (as the Electron
// release is without credentials). With a Developer ID identity in KCHESS_SIGN_IDENTITY it is
// signed with the hardened runtime, and notarized and stapled when APPLE_ID,
// APPLE_APP_SPECIFIC_PASSWORD and APPLE_TEAM_ID are set (the Electron release's variables).
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { join } from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { stageVoiceRuntime } from './voice-runtime.mjs'
import { verifyNoDevBuild } from './dev-build-marker.mjs'

const root = fileURLToPath(new URL('..', import.meta.url))
const args = new Set(process.argv.slice(2))
const final = args.has('--final')
const version = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version
const desktop = JSON.parse(readFileSync(join(root, 'apps/desktop/package.json'), 'utf8')).build
const name = final ? desktop.productName : `${desktop.productName} Preview`
const appId = final ? desktop.appId : `${desktop.appId}.preview`
// The package name (Windows install folder, .deb package) and the Linux and Windows executables:
// the final app's are Electron's, so its installers upgrade Electron's installation in place.
const packageName = final ? 'kchess' : 'kchess-preview'
const executable = {
  win: final ? desktop.productName : name,
  linux: final ? desktop.linux.executableName : `${desktop.linux.executableName}-preview`,
}
// The preview's own NSIS identity, so its installer never touches Electron's installation.
const previewGuid = '3ddfc8df-572e-41e0-a3bd-fc08e5e3da9f'
const platform = { darwin: 'mac', win32: 'win', linux: 'linux' }[process.platform]
const arch = process.arch === 'arm64' ? 'arm64' : 'x64'
if (!platform) throw new Error(`[gpui-package] Unsupported platform ${process.platform}`)
if (platform === 'mac' && arch !== 'arm64')
  throw new Error('[gpui-package] macOS packages are Apple Silicon only.')

const exe = process.platform === 'win32' ? '.exe' : ''
const out = join(root, 'apps/gpui/dist')
const stage = join(root, 'target/gpui-package/stage')
const run = (command, commandArgs, options = {}) =>
  execFileSync(command, commandArgs, { cwd: root, stdio: 'inherit', ...options })
const say = (message) => console.info(`[gpui-package] ${message}`)

if (!args.has('--skip-build')) {
  say('Building the app (release)…')
  // The updater looks for packages under the name this build is made with (apps/gpui/src/updates.rs).
  run('cargo', ['build', '--release', '--locked', '-p', 'kchess-gpui'], {
    env: { ...process.env, KCHESS_PACKAGE_PREFIX: name.replace(/ /g, '-') },
  })
  say('Building the voice helper (release)…')
  run('cargo', ['build', '--release', '--locked', '-p', 'kchess-voice'])
  // The profile's shared core, which every KChess window uses (docs/shared-core.md).
  say('Building the core service (release)…')
  run('cargo', ['build', '--release', '--locked', '-p', 'kchess-service'])
  await stageVoiceRuntime('release')
}
// The app runs voice/kchess-voice beside its own executable, or in the Mac bundle's Resources
// (apps/gpui/src/profile.rs).
const voice = join(root, 'crates/kchess-node/voice')
const voiceFiles = existsSync(voice) ? readdirSync(voice) : []
if (!voiceFiles.some((file) => file.startsWith('kchess-voice')))
  throw new Error('[gpui-package] The voice runtime is not staged: run without --skip-build.')
say('Fetching Stockfish…')
const stockfish = execFileSync(
  'cargo',
  [
    'run',
    '--release',
    '--locked',
    '-q',
    '-p',
    'kchess-core',
    '--example',
    'fetch_stockfish',
    '--',
    join(root, 'target/gpui-package/stockfish'),
  ],
  { cwd: root, stdio: ['ignore', 'pipe', 'inherit'], encoding: 'utf8' },
).trim()
const binary = join(root, `target/release/KChess${exe}`)
// The app starts it from beside its own executable (apps/gpui/src/core.rs).
const service = join(root, `target/release/kchess-service${exe}`)
// Debug builds draw a development badge in the sidebar; a release binary must not contain it.
await verifyNoDevBuild(binary, 'GPUI release')

rmSync(stage, { recursive: true, force: true })
rmSync(out, { recursive: true, force: true })
mkdirSync(stage, { recursive: true })
mkdirSync(out, { recursive: true })
const artifactPrefix = name.replace(/ /g, '-')

/** Copy the voice runtime into `dir`/voice; the paths of its programs and libraries. */
function copyVoice(dir) {
  const target = join(dir, 'voice')
  mkdirSync(target, { recursive: true })
  const code = []
  for (const file of voiceFiles) {
    copyFileSync(join(voice, file), join(target, file))
    if (!file.endsWith('.txt')) {
      if (process.platform !== 'win32') chmodSync(join(target, file), 0o755)
      code.push(join(target, file))
    }
  }
  return code
}

const prepackaged =
  platform === 'mac' ? packageMac() : platform === 'win' ? packageWindows() : packageLinux()
makeInstallers(prepackaged)

const sums = readdirSync(out)
  .filter((file) => statSync(join(out, file)).isFile())
  .sort()
  .map(
    (file) =>
      `${createHash('sha256')
        .update(readFileSync(join(out, file)))
        .digest('hex')}  ${file}`,
  )
writeFileSync(join(out, 'SHA256SUMS.txt'), `${sums.join('\n')}\n`)
say(`Done: ${out}`)
for (const line of sums) console.info(`  ${line}`)

function packageMac() {
  const app = join(stage, `${name}.app`)
  const contents = join(app, 'Contents')
  mkdirSync(join(contents, 'MacOS'), { recursive: true })
  mkdirSync(join(contents, 'Resources'), { recursive: true })
  // The app finds its Stockfish beside its own executable (apps/gpui/src/profile.rs).
  copyFileSync(binary, join(contents, 'MacOS/kchess'))
  copyFileSync(stockfish, join(contents, 'MacOS/stockfish'))
  chmodSync(join(contents, 'MacOS/stockfish'), 0o755)
  copyFileSync(service, join(contents, 'MacOS/kchess-service'))
  chmodSync(join(contents, 'MacOS/kchess-service'), 0o755)
  // Resources, not MacOS: the notices beside the helper are not code, which codesign rejects there.
  const voiceCode = copyVoice(join(contents, 'Resources'))
  copyFileSync(join(root, 'apps/desktop', desktop.mac.icon), join(contents, 'Resources/icon.icns'))
  writeFileSync(join(contents, 'Info.plist'), infoPlist())
  sign(app, [...voiceCode, join(contents, 'MacOS/kchess-service')])
  return app
}

function infoPlist() {
  const entries = {
    CFBundleName: name,
    CFBundleDisplayName: name,
    CFBundleIdentifier: appId,
    CFBundleExecutable: 'kchess',
    CFBundleIconFile: 'icon',
    CFBundlePackageType: 'APPL',
    CFBundleShortVersionString: version,
    CFBundleVersion: version,
    LSApplicationCategoryType: desktop.mac.category,
    LSMinimumSystemVersion: '12.0',
    NSHighResolutionCapable: true,
    NSMicrophoneUsageDescription: desktop.mac.extendInfo.NSMicrophoneUsageDescription,
  }
  const escape = (text) =>
    String(text).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  const body = Object.entries(entries)
    .map(([key, value]) =>
      typeof value === 'boolean'
        ? `  <key>${key}</key>\n  <${value}/>`
        : `  <key>${key}</key>\n  <string>${escape(value)}</string>`,
    )
    .join('\n')
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
${body}
</dict>
</plist>
`
}

/** Sign the nested programs (`nested`, then Stockfish) before the app that seals them. */
function sign(app, nested) {
  const identity = process.env.KCHESS_SIGN_IDENTITY
  const engine = join(app, 'Contents/MacOS/stockfish')
  const inner = [...nested, engine]
  if (!identity) {
    say('Signing ad hoc (no KCHESS_SIGN_IDENTITY)…')
    for (const path of inner) run('codesign', ['--force', '--sign', '-', path])
    run('codesign', ['--force', '--sign', '-', app])
    return
  }
  say(`Signing as ${identity} with the hardened runtime…`)
  // The microphone is for voice input; nothing else needs an exception to the hardened runtime.
  const entitlements = join(stage, 'entitlements.plist')
  writeFileSync(
    entitlements,
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.device.audio-input</key>
  <true/>
</dict>
</plist>
`,
  )
  const flags = ['--force', '--timestamp', '--options', 'runtime', '--sign', identity]
  for (const path of inner) run('codesign', [...flags, path])
  run('codesign', [...flags, '--entitlements', entitlements, app])
  rmSync(entitlements)
  const { APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD, APPLE_TEAM_ID } = process.env
  if (!(APPLE_ID && APPLE_APP_SPECIFIC_PASSWORD && APPLE_TEAM_ID)) {
    throw new Error(
      'Signed Mac packages require APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD and APPLE_TEAM_ID for notarization.',
    )
  }
  say('Notarizing…')
  const upload = join(stage, 'notarize.zip')
  run('ditto', ['-c', '-k', '--keepParent', app, upload])
  run('xcrun', [
    'notarytool',
    'submit',
    upload,
    '--apple-id',
    APPLE_ID,
    '--password',
    APPLE_APP_SPECIFIC_PASSWORD,
    '--team-id',
    APPLE_TEAM_ID,
    '--wait',
  ])
  rmSync(upload)
  run('xcrun', ['stapler', 'staple', app])
}

function packageWindows() {
  const folder = join(stage, name)
  mkdirSync(folder, { recursive: true })
  copyFileSync(binary, join(folder, `${executable.win}.exe`))
  copyFileSync(stockfish, join(folder, 'stockfish.exe'))
  copyFileSync(service, join(folder, 'kchess-service.exe'))
  copyVoice(folder)
  // electron-builder's NSIS installer copies its elevate.exe into Electron's resources folder.
  mkdirSync(join(folder, 'resources'))
  return folder
}

function packageLinux() {
  const folder = join(stage, name)
  mkdirSync(folder, { recursive: true })
  copyFileSync(binary, join(folder, executable.linux))
  copyFileSync(stockfish, join(folder, 'stockfish'))
  copyFileSync(service, join(folder, 'kchess-service'))
  chmodSync(join(folder, executable.linux), 0o755)
  chmodSync(join(folder, 'stockfish'), 0o755)
  chmodSync(join(folder, 'kchess-service'), 0o755)
  copyVoice(folder)
  // electron-builder's .deb writes its AppArmor profile into Electron's resources folder.
  mkdirSync(join(folder, 'resources'))
  return folder
}

/** The installers, made by electron-builder from the staged (and on macOS signed) app. */
function makeInstallers(app) {
  say('Making the installers…')
  // A project of its own: the desktop project's packaging hooks are for the Electron app.
  const project = join(root, 'target/gpui-package/builder')
  rmSync(project, { recursive: true, force: true })
  mkdirSync(project, { recursive: true })
  writeFileSync(
    join(project, 'package.json'),
    JSON.stringify({
      name: packageName,
      version,
      description:
        'A desktop chess app: play Stockfish, play on Lichess, follow friends, review games.',
      author: 'Ketan Singrole',
      license: 'GPL-3.0-or-later',
      homepage: 'https://github.com/ketansingrole/KChess-V2#readme',
    }),
  )
  const resources = join(root, 'apps/desktop/build')
  const config = {
    appId,
    productName: name,
    copyright: desktop.copyright,
    // Only read for Electron's own files, which a prepackaged app has none of.
    electronVersion: JSON.parse(
      readFileSync(join(root, 'apps/desktop/node_modules/electron/package.json'), 'utf8'),
    ).version,
    directories: { output: out, buildResources: resources },
    artifactName: `${artifactPrefix}-\${version}-\${os}-\${arch}.\${ext}`,
    // Signed and notarized above; the zip and disk image keep that signature.
    mac: { target: desktop.mac.target, icon: join(resources, 'icon.icns'), identity: null },
    win: {
      target: desktop.win.target,
      icon: join(resources, 'icon.ico'),
      executableName: executable.win,
    },
    nsis: { ...desktop.nsis, guid: final ? desktop.nsis.guid : previewGuid },
    linux: {
      ...desktop.linux,
      icon: join(resources, 'icon.png'),
      executableName: executable.linux,
    },
    ...(final ? { deb: desktop.deb } : {}),
    publish: final ? desktop.publish : null,
  }
  writeFileSync(join(project, 'electron-builder.json'), JSON.stringify(config, null, 2))
  const cli = createRequire(join(root, 'package.json')).resolve('electron-builder/cli.js')
  // The configured targets fix each platform's architecture, as Electron's release does.
  const target = { mac: '--mac', win: '--win', linux: '--linux' }
  run(process.execPath, [
    cli,
    '--projectDir',
    project,
    '--config',
    'electron-builder.json',
    target[platform],
    '--prepackaged',
    app,
    '--publish',
    'never',
  ])
  // Keep the installers, their block maps and update manifests; drop the builder's working files.
  for (const entry of readdirSync(out)) {
    const keep =
      /\.(dmg|zip|exe|AppImage|deb|blockmap)$/.test(entry) || /^latest.*\.yml$/.test(entry)
    if (!keep) rmSync(join(out, entry), { recursive: true, force: true })
  }
}
