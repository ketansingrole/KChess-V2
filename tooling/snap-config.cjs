// Snap uses the same verified app payload as the other Linux packages.
const { build } = require('../apps/desktop/package.json')

module.exports = {
  ...build,
  linux: { ...build.linux, executableName: 'kchess', target: [{ target: 'snap', arch: ['x64'] }] },
  // Snap Store owns updates; do not generate a GitHub self-update feed for this package.
  publish: null,
  snapcraft: {
    base: 'core24',
    core24: {
      title: 'KChess',
      summary: 'Play, study and improve at chess with Stockfish and Lichess',
      confinement: 'strict',
      grade: 'stable',
      useLXD: true,
      stagePackages: [
        'libnspr4',
        'libnss3',
        'libxss1',
        'libayatana-appindicator3-1',
        'libsecret-1-0',
      ],
      plugs: [
        'desktop',
        'desktop-legacy',
        'home',
        'x11',
        'wayland',
        'network',
        'network-bind',
        'opengl',
        'audio-playback',
        'audio-record',
        'password-manager-service',
        // Without allow-sandbox (which the Store holds for manual review), Electron Builder
        // launches with --no-sandbox and drops chrome-sandbox; strict confinement still applies.
        'browser-support',
      ],
    },
  },
}
