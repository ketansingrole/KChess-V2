import { expect, it } from 'vitest'
import { Linter } from 'eslint'
import architecture from '../../tooling/eslint/architecture.mjs'
import { resolve } from 'node:path'

function lint(code, filename) {
  return new Linter().verify(
    code,
    [
      {
        files: ['**/*.js'],
        plugins: { architecture },
        rules: { 'architecture/boundaries': 'error' },
      },
    ],
    { filename: resolve(filename) },
  )
}
it.each([
  ['apps/desktop/app/page.js', "import { createKChessCore } from '@kchess/native'"],
  [
    'crates/kchess-contracts/ts/test.js',
    "import typeOnly from '../../../apps/desktop/contracts/types'",
  ],
  ['crates/kchess-node/js/test.js', "import { createNodeCore } from '@kchess/node'"],
  [
    'apps/desktop/app/page.js',
    "import { nativeCallSync } from '../../../crates/kchess-node/js/nativeCore'",
  ],
  ['apps/desktop/app/page.js', "export * from '../../../crates/kchess-node/js/store'"],
  ['apps/desktop/app/page.js', "const main = import('../../../crates/kchess-node/js/nativeCore')"],
  ['apps/desktop/app/page.js', "const fs = require('fs')"],
  [
    'apps/desktop/app/page.js',
    "import { nativeCallSync } from '@@/../../crates/kchess-node/js/nativeCore'",
  ],
  [
    'apps/desktop/app/page.js',
    "import { nativeCallSync } from '~/../../../crates/kchess-node/js/nativeCore'",
  ],
  ['apps/desktop/electron/main/test.js', "const electron = require('electron')"],
  ['apps/desktop/electron/main/test.js', "const sqlite = import('node:sqlite')"],
  ['crates/kchess-wasm/js/test.js', "import fs from 'node:fs'"],
  ['crates/kchess-wasm/js/test.js', "import { ref } from 'vue'"],
  ['crates/kchess-wasm/js/test.js', "import { useIntervalFn } from '@vueuse/core'"],
  ['crates/kchess-node/js/test.js', "import { useNuxtApp } from '@nuxt/core'"],
  ['apps/desktop/electron/main/test.js', "import { ipcMain as ipc } from 'electron'"],
  ['apps/desktop/electron/main/test.js', "import * as electron from 'electron'"],
  [
    'apps/desktop/electron/main/test.js',
    "import { getSettings } from '../../../../crates/kchess-node/js/store'",
  ],
  ['apps/desktop/electron/main/test.js', "import { DatabaseSync } from 'node:sqlite'"],
  ['apps/desktop/electron/preload/test.js', "import { getDb } from '../main/db'"],
  ['apps/desktop/electron/main/test.js', "import { view } from '../../app/view'"],
  ['apps/desktop/app/page.js', "import { createKChessCore } from '../../../crates/kchess-node/js'"],
  ['crates/kchess-wasm/js/test.js', "import { loadData } from '../../kchess-node/js/store'"],
  ['crates/kchess-node/js/test.js', "import { app } from 'electron'"],
  ['crates/kchess-node/js/test.js', "import { createNodeCore } from '../../../hosts/node/src'"],
  ['crates/kchess-node/js/test.js', "import { computed } from 'vue'"],
  [
    'hosts/node/src/test.js',
    "import { nativeCallSync } from '../../../crates/kchess-node/js/nativeCore'",
  ],
  ['apps/cli/src/test.js', "import { useKChessStore } from '../../desktop/app/stores/kchess'"],
  ['apps/desktop/app/page.js', "import { createNodeCore } from '../../../hosts/node/src'"],
  ['crates/kchess-node/js/test.js', "const updater = import('electron-updater')"],
  [
    'crates/kchess-node/js/test.js',
    "import { send } from '../../../apps/desktop/electron/main/ipc'",
  ],
  ['crates/kchess-node/js/test.js', "import { view } from '../../../apps/desktop/app/view'"],
  ['crates/kchess-node/js/test.js', "import { DatabaseSync } from 'node:sqlite'"],
  ['tests/core/test.js', "import { DatabaseSync } from 'node:sqlite'"],
  ['apps/desktop/app/page.js', "import { logDebug } from '@kchess/native/logger'"],
  ['apps/desktop/app/page.js', "import { parseFen } from '../../../crates/kchess-wasm/js/chess'"],
  [
    'apps/desktop/electron/main/test.js',
    "import { assertTheme } from '../../../../crates/kchess-wasm/js/validate'",
  ],
  [
    'apps/desktop/contracts/test.js',
    "import typeOnly from '../../../crates/kchess-contracts/ts/types'",
  ],
])('rejects forbidden dependency in %s: %s', (filename, code) => {
  expect(lint(code, filename)).toHaveLength(1)
})
it.each([
  ['apps/desktop/app/page.js', "import { INITIAL_FEN } from '@kchess/rules/position'"],
  ['apps/desktop/app/page.js', "import { IPC_CHANNELS } from '../contracts/ipc'"],
  ['apps/desktop/electron/preload/index.js', "import { ipcRenderer } from 'electron'"],
  ['apps/desktop/electron/main/ipc.ts', "import { ipcMain } from 'electron'"],
  ['apps/desktop/app/page.js', "import { parseFen } from '@kchess/rules/chess'"],
  ['apps/desktop/app/page.js', "import typeOnly from '@kchess/contracts/types'"],
  ['apps/desktop/electron/main/test.js', "import { createKChessCore } from '@kchess/native'"],
  ['apps/desktop/electron/main/test.js', "import { logDebug } from '@kchess/native/logger'"],
  ['apps/desktop/electron/main/test.js', "import { assertTheme } from '@kchess/rules/validate'"],
  ['apps/desktop/electron/main/test.js', "import typeOnly from '@kchess/contracts/types'"],
])('allows owned dependency in %s', (filename, code) => {
  // Rule ownership uses the actual TS path while ESLint parses plain JS test snippets.
  const messages = new Linter().verify(
    code,
    [
      {
        files: ['**/*.{js,ts}'],
        plugins: { architecture },
        rules: { 'architecture/boundaries': 'error' },
      },
    ],
    { filename: resolve(filename) },
  )
  expect(messages).toEqual([])
})
