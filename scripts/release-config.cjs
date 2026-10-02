// Developer ID signing enables Squirrel.Mac updates. Without credentials keep the ad-hoc
// release, with update checks and manual installation rather than a broken install action.
const { build } = require('../package.json')
const signedMac = process.platform === 'darwin' && Boolean(process.env.CSC_LINK)

if (
  signedMac &&
  !(process.env.APPLE_ID && process.env.APPLE_APP_SPECIFIC_PASSWORD && process.env.APPLE_TEAM_ID)
) {
  throw new Error(
    'Signed Mac releases require APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD and APPLE_TEAM_ID for notarization.',
  )
}

// Remove the local packaging config's identity:null so the imported Developer ID is discovered.
const mac = { ...build.mac }
delete mac.identity

module.exports = {
  ...build,
  extraMetadata: { kchessMacAutoUpdate: signedMac },
  mac: signedMac
    ? {
        ...mac,
        type: 'distribution',
        hardenedRuntime: true,
        forceCodeSigning: true,
        notarize: true,
      }
    : { ...mac, identity: '-', hardenedRuntime: false },
}
