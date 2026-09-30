// electron-builder settings for published releases: the package.json config plus ad-hoc macOS
// signing. Without any signature, macOS on Apple Silicon calls a downloaded app "damaged"; ad-hoc
// signed, it asks once in System Settings › Privacy & Security instead. (Real Developer ID signing
// and notarization would remove even that prompt.)
const { build } = require('../package.json')

module.exports = {
  ...build,
  mac: { ...build.mac, identity: '-', hardenedRuntime: false },
}
