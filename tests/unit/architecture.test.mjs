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
  ['apps/desktop/app/page.js', "import { createKChessCore } from '@kchess/core'"],
  ['core/src/contracts/test.js', "import typeOnly from '../../../apps/desktop/contracts/types'"],
  ['core/src/services/test.js', "import { createNodeCore } from '@kchess/node'"],
  ['apps/desktop/app/page.js', "import { getDb } from '../../../core/src/services/db'"],
  ['apps/desktop/app/page.js', "export * from '../../../core/src/services/store'"],
  ['apps/desktop/app/page.js', "const main = import('../../../core/src/services/db')"],
  ['apps/desktop/app/page.js', "const fs = require('fs')"],
  ['apps/desktop/app/page.js', "import { getDb } from '@@/../../core/src/services/db'"],
  ['apps/desktop/app/page.js', "import { getDb } from '~/../../../core/src/services/db'"],
  ['apps/desktop/electron/main/test.js', "const electron = require('electron')"],
  ['apps/desktop/electron/main/test.js', "const sqlite = import('node:sqlite')"],
  ['core/src/domain/test.js', "import fs from 'node:fs'"],
  ['core/src/domain/test.js', "import { ref } from 'vue'"],
  ['core/src/domain/test.js', "import { useIntervalFn } from '@vueuse/core'"],
  ['core/src/services/test.js', "import { useNuxtApp } from '@nuxt/core'"],
  ['apps/desktop/electron/main/test.js', "import { ipcMain as ipc } from 'electron'"],
  ['apps/desktop/electron/main/test.js', "import * as electron from 'electron'"],
  [
    'apps/desktop/electron/main/test.js',
    "import { queryPuzzles } from '../../../../core/src/services/puzzleQueries'",
  ],
  [
    'apps/desktop/electron/main/test.js',
    "import { getSettings } from '../../../../core/src/services/store'",
  ],
  ['apps/desktop/electron/main/test.js', "import { DatabaseSync } from 'node:sqlite'"],
  ['apps/desktop/electron/preload/test.js', "import { getDb } from '../main/db'"],
  ['apps/desktop/electron/main/test.js', "import { view } from '../../app/view'"],
  ['apps/desktop/app/page.js', "import { createKChessCore } from '../../../core/src/services'"],
  ['core/src/domain/test.js', "import { loadData } from '../services/store'"],
  ['core/src/services/test.js', "import { app } from 'electron'"],
  ['core/src/services/test.js', "import { createNodeCore } from '../../../hosts/node/src'"],
  ['core/src/services/test.js', "import { computed } from 'vue'"],
  ['hosts/node/src/test.js', "import { getDb } from '../../../core/src/services/db'"],
  ['apps/cli/src/test.js', "import { useKChessStore } from '../../desktop/app/stores/kchess'"],
  ['apps/desktop/app/page.js', "import { createNodeCore } from '../../../hosts/node/src'"],
  ['core/src/services/test.js', "const updater = import('electron-updater')"],
  ['core/src/services/test.js', "import { send } from '../../../apps/desktop/electron/main/ipc'"],
  ['core/src/services/test.js', "import { view } from '../../../apps/desktop/app/view'"],
  ['core/src/services/test.js', "import { queryPuzzles } from './puzzleQueries'"],
  ['core/src/services/test.js', "import { DatabaseSync } from 'node:sqlite'"],
  ['apps/desktop/app/page.js', "import { logDebug } from '@kchess/core/logger'"],
  ['apps/desktop/app/page.js', "import { parseFen } from '../../../core/src/domain/chess'"],
  [
    'apps/desktop/electron/main/test.js',
    "import { assertTheme } from '../../../../core/src/domain/validate'",
  ],
  ['apps/desktop/contracts/test.js', "import typeOnly from '../../../core/src/contracts/types'"],
])('rejects forbidden dependency in %s: %s', (filename, code) => {
  expect(lint(code, filename)).toHaveLength(1)
})
it.each([
  ['apps/desktop/app/page.js', "import { INITIAL_FEN } from '@kchess/core/domain/position'"],
  ['apps/desktop/app/page.js', "import { IPC_CHANNELS } from '../contracts/ipc'"],
  ['apps/desktop/electron/preload/index.js', "import { ipcRenderer } from 'electron'"],
  ['apps/desktop/electron/main/ipc.ts', "import { ipcMain } from 'electron'"],
  ['core/src/services/puzzleWorker.ts', "import { queryPuzzles } from './puzzleQueries'"],
  ['apps/desktop/app/page.js', "import { parseFen } from '@kchess/core/domain/chess'"],
  ['apps/desktop/app/page.js', "import typeOnly from '@kchess/core/contracts/types'"],
  ['apps/desktop/electron/main/test.js', "import { createKChessCore } from '@kchess/core'"],
  ['apps/desktop/electron/main/test.js', "import { logDebug } from '@kchess/core/logger'"],
  [
    'apps/desktop/electron/main/test.js',
    "import { assertTheme } from '@kchess/core/domain/validate'",
  ],
  ['apps/desktop/electron/main/test.js', "import typeOnly from '@kchess/core/contracts/types'"],
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
