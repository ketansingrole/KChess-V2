'use strict'

// One prebuilt module per host: kchess-native.<platform>-<arch>.node, built by
// `pnpm run build:native` (tooling/build-native.mjs) and shipped unpacked with the app.
const { join } = require('node:path')

module.exports = require(join(__dirname, `kchess-native.${process.platform}-${process.arch}.node`))
