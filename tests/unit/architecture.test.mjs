import { expect, it } from 'vitest'
import { Linter } from 'eslint'
import architecture from '../../scripts/eslint/architecture.mjs'
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
  ['app/page.js', "import { getDb } from '../src/main/db'"],
  ['app/page.js', "export * from '../src/main/store'"],
  ['app/page.js', "const main = import('../src/main/db')"],
  ['app/page.js', "const fs = require('fs')"],
  ['app/page.js', "import { getDb } from '@@/src/main/db'"],
  ['app/page.js', "import { getDb } from '~/../src/main/db'"],
  ['src/main/test.js', "const electron = require('electron')"],
  ['src/main/test.js', "const sqlite = import('node:sqlite')"],
  ['src/shared/test.js', "import fs from 'node:fs'"],
  ['src/main/test.js', "import { ipcMain as ipc } from 'electron'"],
  ['src/main/test.js', "import * as electron from 'electron'"],
  ['src/main/test.js', "import { queryPuzzles } from './puzzleQueries'"],
  ['src/main/test.js', "import { DatabaseSync } from 'node:sqlite'"],
  ['src/preload/test.js', "import { getDb } from '../main/db'"],
  ['src/main/test.js', "import { view } from '../../app/view'"],
])('rejects forbidden dependency in %s: %s', (filename, code) => {
  expect(lint(code, filename)).toHaveLength(1)
})
it.each([
  ['app/page.js', "import { INITIAL_FEN } from 'chessops/fen'"],
  ['app/page.js', "import { IPC_CHANNELS } from '../src/shared/ipc'"],
  ['src/preload/index.js', "import { ipcRenderer } from 'electron'"],
  ['src/main/ipc.ts', "import { ipcMain } from 'electron'"],
  ['src/main/puzzleWorker.ts', "import { queryPuzzles } from './puzzleQueries'"],
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
