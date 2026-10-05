import {
  test as base,
  expect,
  _electron,
  type ElectronApplication,
  type Page,
} from '@playwright/test'
import { cp, mkdtemp, rm, readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { migrate } from '../../src/main/migrations'
import { storeSample } from '../../src/main/puzzleQueries'
import { reviewKey, summarize } from '../../src/shared/review'
import type { StoredReview } from '../../src/shared/types'

const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
const SCHOLARS_PGN = '1. e4 e5 2. Qh5 Nc6 3. Bc4 Nf6 4. Qxf7# 1-0'
const SCHOLARS_UCI = ['e2e4', 'e7e5', 'd1h5', 'b8c6', 'f1c4', 'g8f6', 'h5f7']

/** A synced Lichess game (old enough to stay out of automatic reviews) and its stored review. */
function seedReviewedGame(db: DatabaseSync): void {
  db.prepare('INSERT INTO accounts (username, connected) VALUES (?, 0)').run('tester')
  db.prepare(
    `INSERT INTO games (account, id, createdAt, lastMoveAt, rated, speed, perf, status, winner,
      color, opponent, moves) VALUES (?, ?, ?, ?, 1, 'blitz', 'blitz', 'mate', 'white', 'black', ?, ?)`,
  ).run(
    'tester',
    'scholar1',
    1_600_000_000_000,
    1_600_000_000_000,
    'rival',
    'e4 e5 Qh5 Nc6 Bc4 Nf6 Qxf7#',
  )
  const review: StoredReview = {
    key: reviewKey(START_FEN, SCHOLARS_UCI),
    fen: START_FEN,
    moves: SCHOLARS_UCI,
    source: 'local',
    evals: [
      { cp: 20, depth: 18 },
      { cp: 30, depth: 18 },
      { cp: 20, depth: 18 },
      { cp: 20, depth: 18 },
      { cp: 30, depth: 18 },
      { cp: 30, depth: 18, best: 'g7g6', pv: ['g7g6'] },
      { mate: 1, depth: 18 },
      null,
    ],
    depth: 18,
    complete: true,
    updatedAt: 0,
    gameId: 'scholar1',
  }
  db.prepare(
    `INSERT INTO reviews (key, gameId, source, complete, depth, data, summary, updatedAt)
     VALUES (?, ?, 'local', 1, 18, ?, ?, 0)`,
  ).run(review.key, 'scholar1', JSON.stringify(review), JSON.stringify(summarize(review)))
}

const test = base.extend<{ desktop: { app: ElectronApplication; page: Page; profile: string } }>({
  desktop: async ({ playwright: _playwright }, use, testInfo) => {
    const profile = await mkdtemp(join(tmpdir(), 'kchess-e2e-'))
    if (testInfo.title.includes('voice'))
      await cp(join(process.cwd(), '.data', 'voice', 'cache'), join(profile, 'voice'), {
        recursive: true,
      })
    const db = new DatabaseSync(join(profile, 'kchess.db'))
    migrate(db)
    storeSample(db, [
      {
        id: 'promo123',
        fen: '6k1/P6p/8/8/8/8/5PPP/6K1 b - - 0 1',
        moves: 'h7h6 a7a8q',
        rating: 1500,
        themes: 'promotion',
      },
    ])
    if (testInfo.title.includes('reviewed game')) seedReviewedGame(db)
    if (testInfo.title.includes('paged history')) {
      db.exec("INSERT INTO accounts (username, connected) VALUES ('tester', 0)")
      const insert =
        db.prepare(`INSERT INTO games (account, id, createdAt, lastMoveAt, rated, speed, perf, status, winner, color, opponent, moves, pgn)
        VALUES ('tester', ?, ?, ?, 1, 'blitz', 'blitz', 'mate', ?, 'white', ?, 'e4 e5', '1. e4 e5 *')`)
      for (let i = 0; i < 41; i++)
        insert.run(
          `Paged${String(i).padStart(3, '0')}`,
          1_600_000_000_000 + i,
          1_600_000_000_000 + i,
          i % 2 ? 'black' : 'white',
          `rival${i}`,
        )
    }
    if (testInfo.title.includes('saved position lookup')) {
      const saved = {
        kind: 'opening',
        fen: START_FEN,
        fetchedAt: 1,
        moves: [{ uci: 'e2e4', san: 'e4', white: 100, draws: 20, black: 50 }],
      }
      db.prepare('INSERT INTO position_lookups (key, data, fetchedAt) VALUES (?, ?, ?)').run(
        'opening:' + START_FEN,
        JSON.stringify(saved),
        1,
      )
    }
    db.close()
    let app: ElectronApplication | undefined
    try {
      const executablePath = process.env.ELECTRON_EXEC_PATH
      if (!executablePath)
        throw new Error(
          'Use npm run test:e2e or npm run test:packaged to launch through the branded wrapper.',
        )
      app = await _electron.launch({
        executablePath,
        args: [
          ...(process.env.KCHESS_PACKAGED_EXEC_PATH ? [] : ['.']),
          ...(testInfo.title.includes('voice')
            ? ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream']
            : []),
        ],
        env: { ...process.env, KCHESS_NUXT_URL: '', KCHESS_USER_DATA_DIR: profile },
      })
      await app.context().tracing.start({ screenshots: true, snapshots: true })
      const page = await app.firstWindow()
      page.on('console', (message) => {
        if (message.type() === 'error') console.error(`[renderer] ${message.text()}`)
      })
      page.on('pageerror', (error) => console.error(`[renderer] ${error.message}`))
      await page.waitForSelector('.main-area')
      await expect(page.locator('aside').getByText('Dashboard', { exact: true })).toBeAttached()
      await use({ app, page, profile })
    } finally {
      if (app) {
        if (testInfo.status !== testInfo.expectedStatus) {
          try {
            await app
              .firstWindow()
              .then((page) => page.screenshot({ path: testInfo.outputPath('desktop.png') }))
          } catch {
            /* Window may have crashed. */
          }
        }
        await app
          .context()
          .tracing.stop(
            testInfo.status !== testInfo.expectedStatus
              ? { path: testInfo.outputPath('trace.zip') }
              : {},
          )
        await app.close()
      }
      await rm(profile, { recursive: true, force: true })
    }
  },
})

test('loads cached voice recognition offline and releases the microphone after a training run @packaged', async ({
  desktop: { app, page },
}) => {
  await app.evaluate(() => {
    globalThis.fetch = () =>
      Promise.reject(new Error('Offline voice test: network is unavailable.'))
  })
  await navigate(page, 'Practice')
  await page.getByRole('tab', { name: 'Say the square' }).click()
  await page.getByRole('switch', { name: 'Voice input', exact: true }).click()
  await expect(page.getByText('Ready for your next turn or run', { exact: true })).toBeVisible({
    timeout: 60_000,
  })
  await page.getByRole('button', { name: 'Start', exact: true }).click()
  await expect(page.getByText('Listening…', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'End run', exact: true }).click()
  await expect(page.getByText('Ready for your next turn or run', { exact: true })).toBeVisible()
  await navigate(page, 'Play with Computer')
  // The game page shows the compact voice bar: a toggle button rather than a switch.
  const voice = page.getByRole('button', { name: 'Voice input', exact: true })
  const bar = page.locator('.voice-bar')
  await voice.click()
  await expect(bar).toHaveAttribute('data-state', 'listening', { timeout: 60_000 })
  await voice.click()
  await expect(bar).toHaveAttribute('data-state', 'off')
})

async function navigate(page: Page, label: string) {
  const link = page.locator('aside').getByText(label, { exact: true })
  if (await link.isVisible()) return link.click()
  // Below 1024px (e.g. the 1024×768 Windows CI screen) the sidebar is a slide-over menu instead.
  const menu = page.getByRole('dialog')
  if (!(await menu.isVisible())) await page.getByRole('button', { name: 'Show sidebar' }).click()
  await menu.getByText(label, { exact: true }).click()
  await expect(menu).toBeHidden()
}

/** Click one square; in the board editor this puts down the chosen spare piece. */
async function clickSquare(page: Page, square: string) {
  const bounds = await page.locator('cg-board').boundingBox()
  if (!bounds) throw new Error('Board is not visible')
  const file = square.charCodeAt(0) - 97
  const rank = Number(square[1]) - 1
  await page.mouse.click(
    bounds.x + ((file + 0.5) * bounds.width) / 8,
    bounds.y + ((7 - rank + 0.5) * bounds.height) / 8,
  )
}

/** Click board squares through Chessground's real pointer handlers. */
async function move(page: Page, from: string, to: string) {
  await clickSquare(page, from)
  await clickSquare(page, to)
}

test('launches with SQLite, sandboxed preload and bundled Stockfish @packaged', async ({
  desktop: { app, page, profile },
}) => {
  const runtime = await app.evaluate(({ app }) => ({
    version: process.versions.electron,
    packaged: app.isPackaged,
    profile: app.getPath('userData'),
  }))
  const expectedVersion = JSON.parse(
    await readFile(new URL('../../node_modules/electron/package.json', import.meta.url), 'utf8'),
  ).version
  expect(runtime.version).toBe(expectedVersion)
  expect(runtime.profile).toBe(profile)
  if (process.env.KCHESS_PACKAGED_EXEC_PATH) expect(runtime.packaged).toBe(true)
  const status = await page.evaluate(async () => {
    const data = await window.kchess.loadData()
    const engine = await window.kchess.engineStatus()
    const move = await window.kchess.bestMove([], 'casual', { movetime: 100 })
    return { accounts: data.accounts, engine, move, nodeExposed: 'require' in window }
  })
  expect(status.accounts).toEqual([])
  expect(status.engine.ready).toBe(true)
  expect(status.engine.bundled).toBe(true)
  expect(status.move).toMatch(/^[a-h][1-8][a-h][1-8][qrbn]?$/)
  expect(status.nodeExposed).toBe(false)
  const policies = await page.evaluate(async () => {
    const document = await fetch('kchess://app/index.html')
    const worker = await fetch('kchess://app/_nuxt/vosk-worker.js')
    return {
      document: document.headers.get('Content-Security-Policy'),
      worker: worker.headers.get('Content-Security-Policy'),
    }
  })
  expect(policies.document).not.toContain("'unsafe-eval'")
  expect(policies.document).not.toContain("script-src 'unsafe-inline'")
  expect(policies.worker).toContain("'unsafe-eval'")
})

test('plays keyboard moves and restores an annotated saved study after reload @packaged', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Analysis board')
  const input = page.getByRole('textbox', { name: 'Enter a chess move in SAN or UCI' })
  await input.fill('e4')
  await input.press('Enter')
  await expect(page.getByRole('list', { name: 'Moves', exact: true })).toContainText('e4')
  await input.fill('e7e5')
  await input.press('Enter')
  await expect(page.getByRole('list', { name: 'Moves', exact: true })).toContainText('e5')
  await page.getByText('Study details and saved studies', { exact: true }).click()
  await page.getByLabel('White', { exact: true }).fill('Alice')
  await page.getByLabel('Comment on this position', { exact: true }).fill('Keep this annotation.')
  await page.getByRole('textbox', { name: 'Study name', exact: true }).fill('Opening study')
  await page.getByRole('button', { name: 'Save study', exact: true }).click()
  await expect(page.getByText('Study saved on this device.', { exact: true })).toBeVisible()
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem('kchess:studies:v1')))
    .toContain('Opening study')
  await page.reload()
  await expect(page.getByRole('list', { name: 'Moves', exact: true })).toContainText('e5')
  await page.getByText('Study details and saved studies', { exact: true }).click()
  await expect(page.getByLabel('White', { exact: true })).toHaveValue('Alice')
  await expect(page.getByLabel('Comment on this position', { exact: true })).toHaveValue(
    'Keep this annotation.',
  )
  await expect(page.getByRole('button', { name: 'Opening study', exact: true })).toBeVisible()
  await page.screenshot({ path: join(process.cwd(), 'test-results', 'study-library.png') })
})

test('turns a reviewed game mistake into a scheduled practice position @packaged', async ({
  desktop: { page },
}) => {
  await navigate(page, 'History')
  await page.getByRole('button', { name: /against rival/ }).click()
  await page.getByRole('button', { name: 'Review', exact: true }).click()
  await page.getByRole('button', { name: "Practice Black's mistakes", exact: true }).click()
  await expect(page.getByText(/positions added to Practice/)).toBeVisible()
  await navigate(page, 'Practice')
  await page.getByRole('tab', { name: 'Your mistakes', exact: true }).click()
  await page.getByRole('button', { name: 'Start practice', exact: true }).click()
  await expect(page.locator('cg-board')).toBeVisible()
  await page.getByRole('button', { name: 'Show engine line', exact: true }).click()
  await expect(
    page.getByText('Position completed. Your next revisit has been scheduled.', { exact: true }),
  ).toBeVisible()
})

test('recovers a saved position lookup offline and rejects unsupported tablebases @packaged', async ({
  desktop: { app, page },
}) => {
  await app.evaluate(() => {
    globalThis.fetch = async () => {
      throw new Error('offline')
    }
  })
  await navigate(page, 'Analysis board')
  await page.getByText('Opening explorer and tablebases', { exact: true }).click()
  await page.getByRole('button', { name: 'Look up this position', exact: true }).click()
  await expect(page.getByText(/Could not refresh. Showing the saved lookup/)).toBeVisible()
  await expect(page.getByTitle('White 100 · Draw 20 · Black 50', { exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Tablebase', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('seven pieces')
})

test('plays a move through the board and keeps the game when navigating', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Play with Computer')
  await expect(page.getByText('Your move', { exact: true })).toBeVisible()
  await move(page, 'e2', 'e4')
  await expect(page.getByRole('list', { name: 'Moves', exact: true })).toContainText('e4')
  await expect(page.getByText('Your move', { exact: true })).toBeVisible()
  await navigate(page, 'History')
  await navigate(page, 'Play with Computer')
  await expect(page.getByRole('list', { name: 'Moves', exact: true })).toContainText('e4')
})

test('keeps a failed offline puzzle verdict on remount and opens the promotion picker @packaged', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Puzzles')
  await page.getByRole('tab', { name: 'Offline', exact: true }).click()
  await expect(page.locator('cg-board')).toBeVisible()
  await move(page, 'f2', 'f3')
  await expect(page.getByText('That’s not the move — try again', { exact: true })).toBeVisible()
  await navigate(page, 'History')
  await navigate(page, 'Puzzles')
  await expect(page.locator('cg-board')).toBeVisible()
  await move(page, 'a7', 'a8')
  const picker = page.getByRole('dialog', { name: 'Choose promotion piece' })
  await expect(picker).toBeVisible()
  await expect(picker.getByRole('button', { name: 'Queen', exact: true })).toBeFocused()
  await page.keyboard.press('Shift+Tab')
  await expect(picker.getByRole('button', { name: 'Knight', exact: true })).toBeFocused()
  await page.keyboard.press('Tab')
  await expect(picker.getByRole('button', { name: 'Queen', exact: true })).toBeFocused()
  await picker.getByRole('button', { name: 'Queen', exact: true }).click()
  await expect(
    page.getByText('Solved after a mistake — not counted', { exact: true }),
  ).toBeVisible()
})

test('persists appearance settings after reloading', async ({ desktop: { page } }) => {
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+,' : 'Control+,')
  await expect(
    page.getByRole('heading', { name: 'Appearance', exact: true, level: 1 }),
  ).toBeVisible()
  await page.getByRole('tab', { name: 'Dark', exact: true }).click()
  await expect
    .poll(async () =>
      page.evaluate(async () => (await window.kchess.loadData()).settings.appearance),
    )
    .toBe('dark')
  await page.reload()
  await expect(page.locator('html')).toHaveClass(/dark/)
  expect(
    await page.evaluate(async () => (await window.kchess.loadData()).settings.appearance),
  ).toBe('dark')
})

test('shows update capabilities and persists update preferences @packaged', async ({
  desktop: { app, page },
}) => {
  const status = await page.evaluate(() => window.kchess.appUpdateStatus())
  expect(status.currentVersion).toBe(await app.evaluate(({ app }) => app.getVersion()))
  if (status.canCheck) expect(status.phase).not.toBe('disabled')
  else expect(status.phase).toBe('disabled')
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+,' : 'Control+,')
  await expect(
    page.getByRole('heading', { name: 'Appearance', exact: true, level: 1 }),
  ).toBeVisible()
  await navigate(page, 'Updates')
  await expect(page.getByRole('heading', { name: 'Updates', level: 1 })).toBeVisible()
  await expect(
    page.getByText(`Installed version ${status.currentVersion} · Stable releases`),
  ).toBeVisible()
  const check = page.getByRole('button', { name: 'Check for updates', exact: true })
  const download = page.getByRole('switch', {
    name: 'Download updates in the background',
    exact: true,
  })
  if (status.canCheck) await expect(check).toBeEnabled()
  else await expect(check).toBeDisabled()
  if (status.canInstall) await expect(download).toBeEnabled()
  else await expect(download).toBeDisabled()
  if (status.canCheck) {
    await page.getByRole('switch', { name: 'Automatically check for updates', exact: true }).click()
    await expect
      .poll(() =>
        page.evaluate(async () => (await window.kchess.loadData()).settings.updateAutoCheck),
      )
      .toBe(false)
  }
  // Exercise the actual preload, IPC validation and SQLite round trip without downloading a release.
  await page.evaluate(async () => {
    const { settings } = await window.kchess.loadData()
    await window.kchess.saveSettings({
      ...settings,
      updateAutoCheck: false,
      updateAutoDownload: false,
      updateInstallOnQuit: false,
    })
  })
  await page.reload()
  await expect(
    page.getByRole('heading', { name: 'Appearance', exact: true, level: 1 }),
  ).toBeVisible()
  await navigate(page, 'Updates')
  await expect(
    page.getByRole('switch', { name: 'Automatically check for updates', exact: true }),
  ).not.toBeChecked()
  await expect(download).not.toBeChecked()
  await expect(
    page.getByRole('switch', { name: 'Install updates when I quit', exact: true }),
  ).not.toBeChecked()
  await expect(page.evaluate(() => window.kchess.installAppUpdate())).rejects.toThrow(
    'Download an update',
  )
  await page.screenshot({ path: 'test-results/app-updates.png' })
})

test('exports redacted diagnostics through Settings', async ({
  desktop: { app, page, profile },
}) => {
  const destination = join(profile, 'diagnostics.json')
  await app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath })
    console.error('Diagnostic test: Bearer test-oauth-secret')
  }, destination)
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+,' : 'Control+,')
  await expect(
    page.getByRole('heading', { name: 'Appearance', exact: true, level: 1 }),
  ).toBeVisible()
  await navigate(page, 'Data & storage')
  await page.getByRole('button', { name: 'Export diagnostics', exact: true }).click()
  await expect(page.getByText('Diagnostics exported.', { exact: true })).toBeVisible()
  const report = await readFile(destination, 'utf8')
  expect(report).toContain('Bearer [redacted]')
  expect(report).not.toContain('test-oauth-secret')
  expect(Object.keys(JSON.parse(report))).toEqual([
    'generatedAt',
    'version',
    'platform',
    'arch',
    'versions',
    'logs',
    'performance',
  ])
  expect(JSON.parse(report).performance).toHaveProperty('eventLoopMs')
})

test('sets up a position in the board editor and analyses it with Stockfish', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Board editor')
  await page.getByRole('button', { name: 'Clear board', exact: true }).click()
  await expect(page.getByText('The board is empty.', { exact: true })).toBeVisible()
  const pieces: [string, string[]][] = [
    ['White king', ['g1']],
    ['White rook', ['a1']],
    ['Black king', ['g8']],
    ['Black pawn', ['f7', 'g7', 'h7']],
  ]
  for (const [piece, squares] of pieces) {
    await page.getByRole('button', { name: piece, exact: true }).click()
    for (const square of squares) await clickSquare(page, square)
  }
  await expect(page.getByLabel('FEN', { exact: true })).toHaveValue(
    '6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1',
  )
  await page.getByRole('button', { name: 'Analyse this position', exact: true }).click()
  await expect(page.locator('.ceval-score')).toHaveText('#1', { timeout: 30_000 })
  await expect(page.locator('.pv-list')).toContainText('Ra8#')
})

test('keeps variations on the analysis board and steps through them', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Analysis board')
  await page.locator('.analysis-panel').getByRole('button', { name: 'Import', exact: true }).click()
  await page
    .getByRole('textbox', { name: 'FEN or PGN' })
    .fill('1. e4 e5 (1... c5 2. Nf3) 2. Nf3 Nc6 *')
  await page.getByRole('dialog').getByRole('button', { name: 'Import', exact: true }).click()
  const moves = page.getByRole('list', { name: 'Moves', exact: true })
  await expect(moves).toContainText('Nc6')
  await moves.getByRole('button', { name: /c5$/ }).click()
  await expect(moves.locator('.tree-move.active')).toHaveText(/c5/)
  await page.keyboard.press('ArrowLeft')
  await expect(moves.locator('.tree-move.active')).toHaveText(/e4/)
  // A move played from a position that already has moves becomes another variation.
  await move(page, 'd7', 'd5')
  await expect(moves.locator('.tree-move.active')).toHaveText(/d5/)
  await expect(moves.locator('.tree-variations')).toContainText('d5')
  await expect(page.locator('.pv-row:not(.pending)').first()).toBeVisible({ timeout: 30_000 })
})

test('reviews a game on the analysis board, labelling the blunder and the better move', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Analysis board')
  await page.locator('.analysis-panel').getByRole('button', { name: 'Import', exact: true }).click()
  await page.getByRole('textbox', { name: 'FEN or PGN' }).fill(SCHOLARS_PGN)
  await page.getByRole('dialog').getByRole('button', { name: 'Import', exact: true }).click()
  const review = page.getByRole('region', { name: 'Game review' })
  await review.getByRole('button', { name: 'Review game', exact: true }).click()
  const moves = page.getByRole('list', { name: 'Moves', exact: true })
  // The quick pass labels the blunder within seconds; the deep pass then finishes the review.
  await expect(moves.locator('.tree-glyph.blunder')).toHaveText('??', { timeout: 30_000 })
  await expect(moves.getByRole('button', { name: /Nf6\?\?$/ })).toBeVisible()
  await expect
    .poll(
      async () =>
        (
          await page.evaluate(([fen, moves]) => window.kchess.reviewGet(fen, moves), [
            START_FEN,
            SCHOLARS_UCI,
          ] as const)
        )?.complete,
      { timeout: 90_000 },
    )
    .toBe(true)
  await expect(review.getByText('Reviewing…')).toBeHidden()
  await expect(review.getByRole('table')).toContainText('%')
  await review.getByRole('button', { name: /1 blunder by Black/ }).click()
  await expect(moves.locator('.tree-move.active')).toHaveText(/Nf6/)
  await expect(review.getByText(/is a blunder\./)).toBeVisible()
  await expect(review.getByText(/Best was/)).toBeVisible()
  await review.getByRole('button', { name: 'Show', exact: true }).click()
  await expect(moves.locator('.tree-variations')).toBeVisible()
})

test('shows a reviewed game’s accuracy in History and opens its review', async ({
  desktop: { page },
}) => {
  await navigate(page, 'History')
  const row = page.getByRole('button', { name: /against rival/ })
  await expect(row).toContainText('%')
  await row.click()
  await expect(page.locator('.review-line')).toContainText('accuracy')
  await page.getByRole('button', { name: 'Review', exact: true }).click()
  const review = page.getByRole('region', { name: 'Game review' })
  await expect(review.getByRole('table')).toContainText('tester')
  await expect(review.getByRole('table')).toContainText('rival')
  await expect(page.locator('.tree-glyph.blunder')).toHaveText('??')
})

test('browses paged history through validated IPC with filters and on-demand PGN', async ({
  desktop: { page },
}) => {
  const metadata = await page.evaluate(() => window.kchess.loadData())
  expect(metadata.gameCount).toBe(41)
  expect(metadata).not.toHaveProperty('games')
  const rows = await page.evaluate(() => window.kchess.gamePage({ offset: 20, limit: 20 }))
  expect(rows.total).toBe(41)
  expect(rows.games).toHaveLength(20)
  expect(rows.games[0]!.pgn).toBeUndefined()
  expect(await page.evaluate(() => window.kchess.gamePgn('tester', 'Paged040'))).toBe('1. e4 e5 *')
  expect(
    await page.evaluate(async () => {
      try {
        await window.kchess.gamePage({ offset: 0, limit: 5000 })
        return false
      } catch {
        return true
      }
    }),
  ).toBe(true)
  await navigate(page, 'History')
  await expect(page.locator('.game-row')).toHaveCount(20)
  await expect(page.locator('.pager')).toContainText('1–20 of 41 games')
  await page.getByRole('button', { name: 'Next page' }).click()
  await expect(page.locator('.pager')).toContainText('21–40 of 41 games')
  await expect(page.locator('.game-row').first()).toContainText('rival20')
  await page.getByRole('tab', { name: 'Wins', exact: true }).click()
  await expect(page.locator('.pager')).toContainText('1–20 of 21 games')
  await expect(page.locator('.game-row').first()).toContainText('rival40')
  await page.locator('.game-row').first().click()
  await expect(page.getByRole('button', { name: 'Review', exact: true })).toBeVisible()
})

test('plays two players on one board, exports the game as a GIF and runs the chess clock', async ({
  desktop: { app, page, profile },
}) => {
  const destination = join(profile, 'game.gif')
  await app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath })
  }, destination)
  await navigate(page, 'Over the board')
  await page.getByRole('tab', { name: 'Shared board', exact: true }).click()
  const input = page.getByRole('textbox', { name: 'Enter a chess move in SAN or UCI' })
  for (const move of ['e4', 'e5', 'Qh5', 'Nc6', 'Bc4', 'Nf6', 'Qxf7#']) {
    await input.fill(move)
    await input.press('Enter')
  }
  await expect(page.getByText('White wins', { exact: true })).toBeVisible()
  await expect(page.getByText('Checkmate', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Export', exact: true }).click()
  await page.getByRole('menuitem', { name: 'Animated GIF of the game' }).click()
  await expect(page.getByText('Saved the GIF', { exact: true })).toBeVisible()
  const gif = await readFile(destination)
  expect(gif.subarray(0, 6).toString('latin1')).toBe('GIF89a')
  await page.getByRole('tab', { name: 'Chess clock', exact: true }).click()
  await page.getByRole('button', { name: /^Opponent's clock/ }).click()
  await page.getByRole('button', { name: /^Your clock/ }).click()
  await expect(page.getByText('1 moves', { exact: true })).toBeVisible()
})

test('starts a Chess960 game against Stockfish with a clock', async ({ desktop: { page } }) => {
  await navigate(page, 'Play with Computer')
  await page.getByRole('button', { name: 'Game options', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('tab', { name: 'Chess960', exact: true }).click()
  await dialog.getByRole('button', { name: 'Start game', exact: true }).click()
  await expect(dialog).toBeHidden()
  await expect(page.getByText('Chess960', { exact: true })).toBeVisible()
})

test('summarises a reviewed game on the Insights page', async ({ desktop: { page } }) => {
  await navigate(page, 'Insights')
  await expect(page.getByText('Games', { exact: true })).toBeVisible()
  await expect(page.getByTitle(/^Checkmate: 1 games/)).toBeVisible()
  await expect(page.getByText(/Accuracy \(1 reviewed games\)/)).toBeVisible()
})
