import { oauthPage } from '../../src/main/oauthPage'
import {
  test as base,
  expect,
  _electron,
  type ElectronApplication,
  type Page,
} from '@playwright/test'
import { cp, mkdtemp, rm, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createHash } from 'node:crypto'
import { zipSync } from 'fflate'
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
  db.prepare('INSERT INTO game_reviews (gameId, reviewKey) VALUES (?, ?)').run(
    'scholar1',
    review.key,
  )
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
    if (testInfo.title.includes('unverified startup'))
      db.prepare('INSERT INTO accounts (username, connected) VALUES (?, 1)').run('tester')
    if (testInfo.title.includes('reviewed game')) seedReviewedGame(db)
    if (testInfo.title.includes('quick logout')) {
      seedReviewedGame(db)
      db.exec(
        "UPDATE accounts SET connected = 1; INSERT INTO accounts (username, connected) VALUES ('second', 1); INSERT INTO tokens VALUES ('tester', 'invalid'), ('second', 'invalid'); INSERT INTO api_cache VALUES ('tester:profile', '{}', 0)",
      )
    }
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
          'Use pnpm run test:e2e or pnpm run test:packaged to launch through the branded wrapper.',
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
      await expect(page.locator('aside').getByText('Home', { exact: true })).toBeAttached()
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

test('starts without an account and keeps settings and downloaded puzzles accessible offline', async ({
  desktop: { page },
}) => {
  await expect(page.getByRole('region', { name: 'Start', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Play the computer', exact: true })).toBeVisible()
  const sidebar = page.locator('aside')
  const settings = sidebar.getByRole('button', { name: 'Settings', exact: true })
  const connect = sidebar.getByRole('button', { name: 'Connect Lichess', exact: true })
  await expect(settings).toBeVisible()
  await expect(connect).toBeVisible()
  const settingsBox = await settings.boundingBox()
  const connectBox = await connect.boundingBox()
  expect(settingsBox!.y).toBeLessThan(connectBox!.y)
  await page.screenshot({ path: test.info().outputPath('kchess-account-free-home.png') })
  await settings.click()
  await expect(sidebar.getByText('Appearance', { exact: true })).toBeVisible()
  await sidebar.getByText('Back', { exact: true }).click()
  await expect(page.getByRole('region', { name: 'Start', exact: true })).toBeVisible()
  await page.evaluate(() => {
    Object.defineProperty(navigator, 'onLine', { configurable: true, get: () => false })
    window.dispatchEvent(new Event('offline'))
  })
  await expect(connect).toBeDisabled()
  await page.getByRole('button', { name: 'Practice puzzles', exact: true }).click()
  await expect(page.getByRole('tab', { name: 'Downloaded', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  )
  await expect(page.locator('.cg-wrap')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Next', exact: true })).toBeEnabled()
  await page.screenshot({ path: test.info().outputPath('kchess-account-free-puzzles.png') })
})

test('browses public tournaments anonymously and asks to connect only when joining', async ({
  desktop: { app, page },
}) => {
  await app.evaluate(() => {
    globalThis.fetch = async (input, init) => {
      const request = new Request(input, init)
      if (request.method !== 'GET' || request.headers.has('authorization'))
        throw new Error('Public browsing must be anonymous and read-only')
      const path = new URL(request.url).pathname
      const common = {
        id: 'Public01',
        fullName: 'Public rapid arena',
        status: 20,
        variant: 'standard',
        rated: true,
        clock: { limit: 600, increment: 0 },
        minutes: 60,
        nbPlayers: 12,
        startsAt: Date.now(),
      }
      if (path === '/api/tournament') return Response.json({ started: [common], created: [] })
      if (path === '/api/tournament/Public01')
        return Response.json({
          ...common,
          secondsToFinish: 1200,
          standing: { players: [{ rank: 1, name: 'TestPlayer', rating: 1800, score: 4 }] },
        })
      throw new Error('Unexpected request: ' + path)
    }
  })
  await navigate(page, 'Tournaments')
  await page
    .locator('.timeline')
    .getByRole('listitem', { name: /Public rapid arena/ })
    .click()
  await expect(
    page.locator('ol').getByRole('listitem').filter({ hasText: 'TestPlayer' }),
  ).toBeVisible()
  await page.getByRole('button', { name: 'Join', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'Connect Lichess to join' })
  await expect(dialog).toBeVisible()
  await dialog.getByRole('button', { name: 'Keep browsing', exact: true }).click()
  await expect(dialog).toBeHidden()
  await expect(
    page.locator('ol').getByRole('listitem').filter({ hasText: 'TestPlayer' }),
  ).toBeVisible()
})

test('offers account-free actions and manages offline downloads without signing in', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Game history')
  await page.getByRole('tab', { name: 'Lichess', exact: true }).click()
  await expect(page.getByText('No saved Lichess games', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Follow a player', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Follow player', exact: true })).toBeVisible()
  await expect(
    page.getByRole('textbox', { name: 'Lichess username to follow', exact: true }),
  ).toBeVisible()
  await navigate(page, 'Lichess insights')
  await expect(page.getByText('No saved Lichess games to explore', { exact: true })).toBeVisible()
  await navigate(page, 'Practice')
  await page.getByRole('tab', { name: 'Your mistakes', exact: true }).click()
  await page.getByRole('button', { name: 'Analyze a game', exact: true }).click()
  await expect(page.locator('cg-board')).toBeVisible()
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+,' : 'Control+,')
  await navigate(page, 'Offline downloads')
  await expect(page.getByText('Included with KChess', { exact: true })).toBeVisible()
  await expect(page.getByText('Download needed', { exact: true })).toBeVisible()
  await expect(
    page.getByRole('button', { name: 'Download voice model', exact: true }),
  ).toBeEnabled()
  await page.evaluate(() => {
    Object.defineProperty(navigator, 'onLine', { configurable: true, get: () => false })
    window.dispatchEvent(new Event('offline'))
  })
  await expect(
    page.getByRole('button', { name: 'Download voice model', exact: true }),
  ).toBeDisabled()
  await expect(
    page.getByRole('button', { name: 'Download a fresh sample', exact: true }),
  ).toBeDisabled()
  await page.screenshot({ path: test.info().outputPath('kchess-offline-downloads.png') })
})

test('loads cached voice recognition offline and releases the microphone after a training run @packaged', async ({
  desktop: { app, page },
}) => {
  await app.evaluate(() => {
    globalThis.fetch = () =>
      Promise.reject(new Error('Offline voice test: network is unavailable.'))
  })
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+,' : 'Control+,')
  await navigate(page, 'Offline downloads')
  await expect(page.getByText(/^Installed ·/)).toBeVisible()
  await expect(
    page.getByRole('button', { name: 'Voice input settings', exact: true }),
  ).toBeVisible()
  await navigate(page, 'Back')
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
  await expect(link).toBeAttached()
  if (await link.isVisible()) return link.click()
  // Below 1024px (e.g. the 1024×768 Windows CI screen) the sidebar is a slide-over menu instead.
  const menu = page.getByRole('dialog')
  if (!(await menu.isVisible())) await page.getByRole('button', { name: 'Show sidebar' }).click()
  await menu.getByText(label, { exact: true }).click()
  await expect(menu).toBeHidden()
}

test('swipes through page history without repeating navigation during momentum', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Board editor')
  await expect(page).toHaveURL(/\/editor/)
  await navigate(page, 'Analysis board')
  await expect(page).toHaveURL(/\/analysis/)
  await page.locator('cg-board').hover()
  // Momentum arrives as one burst: dispatch both wheels in the same task so the
  // tail is processed inside the gesture window no matter how loaded the runner is.
  await page.evaluate(() => {
    const board = document.querySelector('cg-board')!
    for (let i = 0; i < 2; i++)
      board.dispatchEvent(
        new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaX: -120 }),
      )
  })
  await expect(page).toHaveURL(/\/editor/)
  await page.waitForTimeout(350)
  await page.mouse.wheel(120, 0)
  await expect(page).toHaveURL(/\/analysis/)
  await page.waitForTimeout(350)
  await page.mouse.wheel(120, 0)
  await page.waitForTimeout(350)
  await expect(page).toHaveURL(/\/analysis/)
})

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
  await expect(page.getByRole('group', { name: 'Moves', exact: true })).toContainText('e4')
  await input.fill('e7e5')
  await input.press('Enter')
  await expect(page.getByRole('group', { name: 'Moves', exact: true })).toContainText('e5')
  await page.getByRole('button', { name: 'Study actions', exact: true }).click()
  await page.getByRole('menuitem', { name: 'Game details…' }).click()
  await page.getByLabel('White', { exact: true }).fill('Alice')
  await page.keyboard.press('Escape')
  await page.getByRole('button', { name: 'Add a comment' }).click()
  await page.getByLabel('Comment on 1… e5', { exact: true }).fill('Keep this annotation.')
  await page.getByRole('button', { name: 'Mistake', exact: true }).click()
  await page.getByRole('button', { name: 'Save as study', exact: true }).click()
  await page.getByLabel('Study name', { exact: true }).fill('Opening study')
  await page.getByRole('button', { name: 'Save study', exact: true }).click()
  await expect(page.getByText('Study saved', { exact: true })).toBeVisible()
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem('kchess:studies:v1')))
    .toContain('Keep this annotation.')
  await page.reload()
  const moves = page.getByRole('group', { name: 'Moves', exact: true })
  await expect(moves).toContainText('e5?')
  await expect(moves).toContainText('Keep this annotation.')
  await expect(page.getByLabel('Study name', { exact: true })).toHaveValue('Opening study')
  await page.screenshot({ path: join(process.cwd(), 'test-results', 'study-board.png') })
  await navigate(page, 'Studies')
  await expect(page.getByRole('button', { name: 'Open Opening study', exact: true })).toBeVisible()
  await expect(page.getByText('Alice – ?')).toBeVisible()
  const preview = await page.locator('.study-preview cg-board').boundingBox()
  expect(preview?.width).toBeGreaterThan(100)
  expect(preview?.height).toBeGreaterThan(100)
  await page.screenshot({ path: join(process.cwd(), 'test-results', 'study-library.png') })
})

test('turns a reviewed game mistake into a scheduled practice position @packaged', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Game history')
  await page.getByRole('tab', { name: 'Lichess', exact: true }).click()
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
  await page.getByRole('tab', { name: 'Explorer', exact: true }).click()
  await page.getByRole('tab', { name: 'Lichess', exact: true }).click()
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
  await expect(page.getByRole('group', { name: 'Moves', exact: true })).toContainText('e4')
  await expect(page.getByText('Your move', { exact: true })).toBeVisible()
  await navigate(page, 'Home')
  await expect(page.getByRole('button', { name: /^Continue your game\b/ })).toBeVisible()
  await page.reload()
  await expect(page.getByRole('button', { name: /^Continue your game\b/ })).toBeVisible()
  await page.getByRole('button', { name: /^Continue your game\b/ }).click()
  await expect(page.getByRole('group', { name: 'Moves', exact: true })).toContainText('e4')
  await navigate(page, 'Game history')
  await expect(page.locator('.local-history-row')).toHaveCount(1)
  await page.getByRole('button', { name: 'Replay', exact: true }).click()
  await expect(page.locator('cg-board')).toBeVisible()
})

test('keeps a failed offline puzzle verdict on remount and opens the promotion picker @packaged', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Puzzles')
  await page.getByRole('tab', { name: 'Downloaded', exact: true }).click()
  await expect(page.locator('cg-board')).toBeVisible()
  await move(page, 'f2', 'f3')
  await expect(page.getByText('That’s not the move — try again', { exact: true })).toBeVisible()
  await navigate(page, 'Game history')
  await page.getByRole('tab', { name: 'Lichess', exact: true }).click()
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
    page.getByRole('heading', { name: `KChess ${status.currentVersion}`, level: 2 }),
  ).toBeVisible()
  const check = page.getByRole('button', { name: 'Check for updates', exact: true })
  const download = page.getByRole('switch', {
    name: 'Download updates in the background',
    exact: true,
  })
  // Controls that cannot work in this installation are left out rather than shown disabled.
  if (status.canCheck) await expect(check).toBeEnabled()
  else await expect(check).toHaveCount(0)
  if (status.canInstall) await expect(download).toBeEnabled()
  else await expect(download).toHaveCount(0)
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
  if (status.canCheck) {
    await expect(
      page.getByRole('switch', { name: 'Automatically check for updates', exact: true }),
    ).not.toBeChecked()
  }
  if (status.canInstall) {
    await expect(download).not.toBeChecked()
    await expect(
      page.getByRole('switch', { name: 'Install updates when I quit', exact: true }),
    ).not.toBeChecked()
  }
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
    'sessionId',
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
  const moves = page.getByRole('group', { name: 'Moves', exact: true })
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
  await page.getByRole('tab', { name: 'Review', exact: true }).click()
  const review = page.getByRole('region', { name: 'Game review' })
  await review.getByRole('button', { name: 'Review game', exact: true }).click()
  await page.getByRole('tab', { name: /^Moves/ }).click()
  const moves = page.getByRole('group', { name: 'Moves', exact: true })
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
  await page.getByRole('tab', { name: /^Review/ }).click()
  await expect(review.getByText('Reviewing…')).toBeHidden()
  await expect(review.getByRole('table')).toContainText('%')
  await review.getByRole('button', { name: /1 blunder by Black/ }).click()
  await expect(page.locator('.tree-move.active')).toHaveText(/Nf6/)
  await expect(review.getByText(/is a blunder\./)).toBeVisible()
  await expect(review.getByText(/Best was/)).toBeVisible()
  await review.getByRole('button', { name: 'Show', exact: true }).click()
  await page.getByRole('tab', { name: /^Moves/ }).click()
  await expect(moves.locator('.tree-variations')).toBeVisible()
})

test('shows a reviewed game’s accuracy in History and opens its review', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Game history')
  await page.getByRole('tab', { name: 'Lichess', exact: true }).click()
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
  // Publicly followed players are stored, but their games aren't the signed-in user's activity.
  await expect(
    page.getByRole('heading', { name: 'Your Lichess activity', exact: true }),
  ).toBeHidden()
  await expect(page.locator('.stats-grid')).toHaveCount(0)
  await expect(page.getByRole('combobox', { name: 'Account', exact: true })).toBeHidden()
  await expect(page.getByRole('button', { name: 'Sync games', exact: true })).toBeHidden()
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
  await navigate(page, 'Game history')
  await page.getByRole('tab', { name: 'Lichess', exact: true }).click()
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
  // The typed-move box sits in the player's own line, not on a row of its own.
  await expect(page.locator('.player-line .board-move-form')).toBeVisible()
  await page.screenshot({ path: join(process.cwd(), 'test-results', 'board-entry.png') })
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
  await page.getByRole('button', { name: /^Player 1 clock/ }).click()
  await page.getByRole('button', { name: /^Player 2 clock/ }).click()
  await expect(page.getByText('1 moves', { exact: true })).toBeVisible()
  await navigate(page, 'Game history')
  await page.getByRole('tab', { name: 'Over the board', exact: true }).click()
  await expect(page.locator('.local-history-row').filter({ hasText: 'Checkmate' })).toBeVisible()
  await page.reload()
  await page.getByRole('tab', { name: 'Over the board', exact: true }).click()
  await expect(page.locator('.local-history-row').filter({ hasText: 'Checkmate' })).toBeVisible()
  await page.screenshot({
    path: test.info().outputPath('kchess-local-game-history.png'),
    animations: 'disabled',
  })
  await page
    .locator('.local-history-row')
    .filter({ hasText: 'Checkmate' })
    .getByRole('button', { name: 'Replay', exact: true })
    .click()
  await expect(page.locator('cg-board')).toBeVisible()
  await page.getByRole('button', { name: 'Open analysis', exact: true }).click()
  await expect(page.locator('.analysis-panel')).toBeVisible()
})

test('cancels a long GIF export and keeps the renderer responsive while encoding', async ({
  desktop: { app, page, profile },
}) => {
  const destination = join(profile, 'long-game.gif')
  await app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath })
  }, destination)
  // CI screens are small (macOS runners are 1024×768): a long game's moves must stay in their panel.
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.setSize(1024, 740))
  await navigate(page, 'Analysis board')
  await page.locator('.analysis-panel').getByRole('button', { name: 'Import', exact: true }).click()
  const pgn =
    Array.from({ length: 100 }, (_, n) => `${2 * n + 1}. Nf3 Nf6 ${2 * n + 2}. Ng1 Ng8`).join(' ') +
    ' *'
  await page.getByRole('textbox', { name: 'FEN or PGN' }).fill(pgn)
  await page.getByRole('dialog').getByRole('button', { name: 'Import', exact: true }).click()
  const start = async () => {
    await page.getByRole('button', { name: 'Export', exact: true }).click()
    await page.getByRole('menuitem', { name: 'Animated GIF of the game' }).click()
  }
  await start()
  await expect(page.getByRole('status').filter({ hasText: /\d+\/401/ })).toBeVisible()
  await page.getByRole('button', { name: 'Cancel export', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Cancel export', exact: true })).toBeHidden()
  await expect(readFile(destination)).rejects.toMatchObject({ code: 'ENOENT' })
  await page.evaluate(() => {
    const root = document.documentElement
    root.dataset.exportMeasure = 'running'
    root.dataset.exportTicks = '0'
    const tick = () => {
      if (root.dataset.exportMeasure !== 'running') return
      root.dataset.exportTicks = String(Number(root.dataset.exportTicks) + 1)
      requestAnimationFrame(tick)
    }
    requestAnimationFrame(tick)
  })
  await start()
  await expect(page.getByText('Saved the GIF', { exact: true })).toBeVisible({ timeout: 30_000 })
  const ticks = await page.evaluate(() => {
    document.documentElement.dataset.exportMeasure = 'done'
    return Number(document.documentElement.dataset.exportTicks)
  })
  expect(ticks).toBeGreaterThan(3)
  const gif = await readFile(destination)
  expect(gif.subarray(0, 6).toString('latin1')).toBe('GIF89a')
  expect(gif.at(-1)).toBe(0x3b)
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
  await navigate(page, 'Lichess insights')
  await expect(page.getByText('Games', { exact: true })).toBeVisible()
  await expect(page.getByRole('img', { name: /^Checkmate: 1 game/ })).toBeVisible()
  await expect(page.getByText(/Accuracy \(1 reviewed games\)/)).toBeVisible()
})

test('blocks assistance after unverified startup until account status is confirmed @packaged', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Analysis board')
  await expect(
    page.getByText('Analysis paused until Lichess game status is verified', { exact: true }),
  ).toBeVisible()
  await expect(page.locator('.pv-eval')).toHaveCount(0)
  const errors = await page.evaluate(async (fen) => {
    const blocked = async (request: Promise<unknown>) =>
      request.then(
        () => 'allowed',
        (cause: Error) => cause.message,
      )
    return Promise.all([
      blocked(window.kchess.startAnalysis({ fen, lines: 1 })),
      blocked(window.kchess.bestMove([], 'club')),
      blocked(window.kchess.positionLookup('opening', fen)),
      blocked(window.kchess.cloudEval(fen, 1)),
      blocked(window.kchess.reviewRequest({ fen, moves: ['e2e4'] })),
    ])
  }, START_FEN)
  expect(errors).toHaveLength(5)
  expect(errors.every((message) => message.includes('unavailable'))).toBe(true)
  await page.evaluate(async () => {
    await window.kchess.removeAccount('tester')
    await window.kchess.resumeOnline()
  })
  await expect(
    page.getByText('Analysis paused until Lichess game status is verified', { exact: true }),
  ).toHaveCount(0)
  await expect(page.locator('.pv-eval').first()).toBeVisible({ timeout: 20_000 })
})

test('prepares a voice archive with the production worker @packaged', async ({
  desktop: { app, profile },
}) => {
  const archive = join(profile, 'test-model.zip')
  const zip = zipSync({
    'test-model/am/final.mdl': new TextEncoder().encode('production worker fixture'),
  })
  await writeFile(archive, zip)
  const sha256 = createHash('sha256').update(zip).digest('hex')
  const result = await app.evaluate(
    async ({ app }, options) => {
      const { Worker } = process.getBuiltinModule(
        'node:worker_threads',
      ) as typeof import('node:worker_threads')
      const { join } = process.getBuiltinModule('node:path') as typeof import('node:path')
      const worker = new Worker(join(app.getAppPath(), 'out/main/voiceModelWorker.js'), {
        workerData: {
          archive: options.archive,
          directory: options.directory,
          model: { name: 'test-model', sha256: options.sha256 },
        },
      })
      try {
        return await new Promise<{ prepared: string; marker: { source: string; size: number } }>(
          (resolve, reject) => {
            worker.once('message', (message) =>
              message.error ? reject(new Error(message.error)) : resolve(message),
            )
            worker.once('error', reject)
            worker.once('exit', () => reject(new Error('Worker exited without a prepared archive')))
          },
        )
      } finally {
        await worker.terminate()
      }
    },
    { archive, directory: profile, sha256 },
  )
  expect(result.marker.source).toBe(sha256)
  const prepared = await readFile(result.prepared)
  expect(prepared.subarray(0, 2)).toEqual(Buffer.from([0x1f, 0x8b]))
  expect(prepared.length).toBe(result.marker.size)
})

test('renders the Lichess authorization confirmation page clearly', async ({
  desktop: { page },
}) => {
  await page.setContent(oauthPage(true))
  await expect(page.getByRole('heading', { name: 'Signed in to Lichess' })).toBeVisible()
  await expect(
    page.getByText('You can close this browser tab and continue in KChess.'),
  ).toBeVisible()
  await page.screenshot({ path: test.info().outputPath('kchess-lichess-confirmation.png') })
})

test('quick logout removes only the selected account and returns to account-free Home after the last', async ({
  desktop: { page },
}) => {
  const before = await page.evaluate(() => window.kchess.loadData())
  expect(before.accounts.filter((a) => a.connected)).toHaveLength(2)
  const heading = page.getByRole('heading', { name: 'Your Lichess activity', exact: true })
  await expect(heading).toBeVisible()
  const titleBox = await heading.boundingBox()
  const cardsBox = await page.locator('.stats-grid').boundingBox()
  expect(cardsBox!.y - titleBox!.y - titleBox!.height).toBeGreaterThanOrEqual(16)
  await page.screenshot({ path: test.info().outputPath('kchess-home-activity-spacing.png') })
  await navigate(page, 'Over the board')
  await page.getByRole('tab', { name: 'Shared board', exact: true }).click()
  const input = page.getByRole('textbox', { name: 'Enter a chess move in SAN or UCI' })
  await input.fill('e4')
  await input.press('Enter')
  await page.getByRole('button', { name: /account menu/i }).click()
  await page.getByRole('menuitemcheckbox', { name: '@tester', exact: true }).click()
  await page.keyboard.press('Escape')
  await page.getByRole('button', { name: /account menu/i }).click()
  await expect(
    page.getByRole('menuitem', { name: 'Log out all accounts', exact: true }),
  ).toBeVisible()
  const logout = page.getByRole('menuitem', { name: 'Log out @tester', exact: true })
  await expect(logout).toBeVisible()
  await Promise.all([page.waitForEvent('load'), logout.click()])
  await expect(page.getByRole('region', { name: 'Start', exact: true })).toBeVisible()
  const remaining = await page.evaluate(() => window.kchess.loadData())
  expect(remaining.accounts).toMatchObject([{ username: 'second', connected: true }])
  await expect(page.getByRole('button', { name: /account menu/i })).toContainText('@second')
  await page.getByRole('button', { name: /account menu/i }).click()
  await expect(
    page.getByRole('menuitem', { name: 'Log out all accounts', exact: true }),
  ).toBeHidden()
  await Promise.all([
    page.waitForEvent('load'),
    page.getByRole('menuitem', { name: 'Log out', exact: true }).click(),
  ])
  await expect(
    page.locator('aside').getByRole('button', { name: 'Connect Lichess', exact: true }),
  ).toBeVisible()
  await expect(page.getByRole('button', { name: /account menu/i })).toBeHidden()
  const after = await page.evaluate(() => window.kchess.loadData())
  expect(after.accounts).toEqual([])
  expect(after.gameCount).toBe(0)
  expect(after.settings).toEqual(before.settings)
  await navigate(page, 'Game history')
  await page.getByRole('tab', { name: 'Over the board', exact: true }).click()
  await expect(page.locator('.local-history-row')).toHaveCount(1)
  await page.getByRole('tab', { name: 'Lichess', exact: true }).click()
  await expect(page.getByText('No saved Lichess games', { exact: true })).toBeVisible()
  await navigate(page, 'Puzzles')
  await expect(page.getByRole('tab', { name: 'Practice', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  )
  await page.reload()
  expect((await page.evaluate(() => window.kchess.loadData())).accounts).toEqual([])
})

test('quick logout all accounts is a separate explicit action', async ({ desktop: { page } }) => {
  await page.getByRole('button', { name: /account menu/i }).click()
  await Promise.all([
    page.waitForEvent('load'),
    page.getByRole('menuitem', { name: 'Log out all accounts', exact: true }).click(),
  ])
  await expect(
    page.locator('aside').getByRole('button', { name: 'Connect Lichess', exact: true }),
  ).toBeVisible()
  expect((await page.evaluate(() => window.kchess.loadData())).accounts).toEqual([])
})

test('labels search results and keeps error notifications keyboard accessible', async ({
  desktop: { page },
}, testInfo) => {
  // Count repair queries in the real renderer without shipping diagnostic state in the app.
  await page.evaluate(() => {
    const counts = { search: 0, notifications: 0, body: 0 }
    ;(window as unknown as { accessibilityQueries: typeof counts }).accessibilityQueries = counts
    const original = Element.prototype.querySelectorAll
    Element.prototype.querySelectorAll = function (this: Element, selector: string) {
      if (selector === '.kchess-toasts' || selector === '.kchess-search-palette [role="listbox"]') {
        const owner = this.getAttribute('data-ui-accessibility')
        if (owner === 'search' || owner === 'notifications') counts[owner]++
        if (this === document.body) counts.body++
      }
      return original.call(this, selector)
    } as typeof original
  })
  const counts = () =>
    page.evaluate(
      () =>
        (window as unknown as { accessibilityQueries: Record<string, number> })
          .accessibilityQueries,
    )
  await navigate(page, 'Analysis board')
  const engine = page.getByRole('switch', { name: 'Engine analysis', exact: true })
  await expect(engine).toBeEnabled()
  if ((await engine.getAttribute('aria-checked')) !== 'true') await engine.click()
  await expect(page.locator('.pv-row:not(.pending)').first()).toBeVisible({ timeout: 30_000 })
  const moveInput = page.getByRole('textbox', { name: 'Enter a chess move in SAN or UCI' })
  await moveInput.fill('e4')
  await moveInput.press('Enter')
  await expect(page.getByRole('group', { name: 'Moves', exact: true })).toContainText('e4')
  await expect(page.locator('.pv-row:not(.pending)').first()).toBeVisible({ timeout: 30_000 })
  const analysisCounts = await counts()
  expect(analysisCounts).toEqual({ search: 0, notifications: 0, body: 0 })
  const search = page.getByRole('textbox', { name: 'Search pages and commands', exact: true })
  await search.click()
  await expect(page.getByRole('listbox', { name: 'Search results', exact: true })).toBeVisible()
  await search.fill('zzzz-no-results')
  await expect(page.getByRole('listbox', { name: 'Search results', exact: true })).toBeVisible()
  await search.fill('settings')
  await expect(page.getByRole('listbox', { name: 'Search results', exact: true })).toContainText(
    'Settings',
  )
  const searchCounts = await counts()
  expect(searchCounts.search).toBeGreaterThan(0)
  expect(searchCounts.notifications).toBe(0)
  await page.keyboard.press('Escape')
  await navigate(page, 'Following')
  const navigationCounts = await counts()
  expect(navigationCounts.notifications).toBe(0)
  expect(navigationCounts.body).toBe(0)
  await page.getByRole('textbox', { name: 'Lichess username to follow' }).fill('!')
  await page.getByRole('button', { name: 'Follow player', exact: true }).click()
  const notifications = page.locator('.kchess-toasts')
  await expect(notifications.getByText('Something went wrong', { exact: true })).toBeVisible()
  await expect(page.locator('span[aria-hidden="true"][tabindex="0"]')).toHaveCount(0)
  const notificationCounts = await counts()
  expect(notificationCounts.notifications).toBeGreaterThan(0)
  expect(notificationCounts.body).toBe(0)
  await testInfo.attach('accessibility-repair-queries', {
    body: JSON.stringify(
      { analysisCounts, searchCounts, navigationCounts, notificationCounts },
      null,
      2,
    ),
    contentType: 'application/json',
  })
  // The library's F8 shortcut enters the notification area; dismissal remains reachable.
  await page.keyboard.press('F8')
  await page.keyboard.press('Tab')
  await page.keyboard.press('Tab')
  const close = notifications.getByRole('button', { name: 'Close', exact: true })
  await expect(close).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(notifications.getByText('Something went wrong', { exact: true })).toBeHidden()
})

test('announces page navigation and prefetches links on intent @packaged', async ({
  desktop: { app, page },
}) => {
  await expect(page).toHaveTitle('Home — KChess')
  const sidebar = page.locator('aside')
  const link = sidebar.getByRole('link', { name: 'Analysis board', exact: true })
  // Wide desktop sidebar; interaction prefetch must not mount the destination page.
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.setSize(1280, 800))
  await expect(link).toBeVisible()
  await link.focus()
  await expect(link).toHaveClass(/prefetched/)
  await expect(page).toHaveTitle('Home — KChess')
  await link.click()
  await expect(page).toHaveTitle('Analysis board — KChess')
  await expect(page.locator('.nuxt-route-announcer [aria-live="polite"]')).toHaveText(
    'Analysis board — KChess',
  )
  await navigate(page, 'Practice')
  await expect(page).toHaveTitle('Practice — KChess')
  await expect(page.locator('.nuxt-route-announcer [aria-live="polite"]')).toHaveText(
    'Practice — KChess',
  )
})

test('recovers a failed reviewed game Insights page and records route diagnostics @packaged', async ({
  desktop: { app, page, profile },
}) => {
  await navigate(page, 'Lichess insights')
  await expect(page.getByText(/Accuracy \(1 reviewed games\)/)).toBeVisible()
  const corruptInsights = () =>
    page.evaluate(() => {
      const nuxt = (
        window as unknown as {
          useNuxtApp(): { _asyncData: Record<string, { data: { value: unknown } }> }
        }
      ).useNuxtApp()
      // Simulate malformed output reaching a rendering component, through real Nuxt state.
      nuxt._asyncData['insights-report']!.data.value = { total: 1 }
    })
  await corruptInsights()
  await expect(page.getByRole('alert')).toHaveText("This page couldn't load")
  await expect(page.locator('aside').getByText('Analysis board', { exact: true })).toBeAttached()
  await page.getByRole('button', { name: 'Try again', exact: true }).click()
  await expect(page.getByText(/Accuracy \(1 reviewed games\)/)).toBeVisible()
  await corruptInsights()
  await expect(page.getByRole('alert')).toHaveText("This page couldn't load")
  await navigate(page, 'Analysis board')
  await expect(page).toHaveTitle('Analysis board — KChess')
  await expect(page.locator('cg-board')).toBeVisible()
  await expect(page.getByText("This page couldn't load")).toHaveCount(0)
  const destination = join(profile, 'diagnostics.json')
  await app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath })
  }, destination)
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+,' : 'Control+,')
  await navigate(page, 'Data & storage')
  await page.getByRole('button', { name: 'Export diagnostics', exact: true }).click()
  await expect(page.getByText('Diagnostics exported.', { exact: true })).toBeVisible()
  const report = JSON.parse(await readFile(destination, 'utf8'))
  expect(report.logs.join('')).toContain('Renderer page error:')
  expect(report.logs.join('')).toContain('/insights')
  expect(report.performance.operations).toHaveProperty(['page.navigation:/insights'])
})

test('quick pick switches modes, runs commands and remembers them', async ({
  desktop: { page },
}, testInfo) => {
  const results = page.getByRole('listbox', { name: 'Search results', exact: true })
  const input = page.getByRole('textbox', { name: 'Search pages and commands', exact: true })
  await page.keyboard.press('ControlOrMeta+K')
  await expect(input).toBeFocused()
  await expect(results.getByRole('option', { name: /Run a command/ })).toBeVisible()
  await testInfo.attach('quick-pick-open', {
    body: await page.screenshot(),
    contentType: 'image/png',
  })
  // Picking a mode row switches the prefix instead of closing the palette.
  await results.getByRole('option', { name: /Run a command/ }).click()
  await expect(input).toHaveValue('>')
  await input.pressSequentially(' new analysis')
  await expect(results.getByRole('option').first()).toContainText('New analysis board')
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(/\/analysis$/)
  await expect(results).toBeHidden()

  // Shift+Cmd/Ctrl+P opens straight into command mode; Backspace returns to plain search.
  await page.keyboard.press('ControlOrMeta+Shift+P')
  await expect(input).toHaveValue('>')
  await expect(results.getByRole('option', { name: /Flip board/ })).toBeVisible()
  await input.press('Backspace')
  await expect(input).toHaveValue('')
  await expect(results.getByRole('option', { name: /New analysis board/ })).toContainText(
    'recently used',
  )

  // Settings mode shows current state and toggles in place.
  await input.fill('#sound')
  const sound = results.getByRole('option', { name: /^Sound (On|Off)$/ })
  const before = (await sound.textContent())?.includes('On')
  await sound.click()
  await expect(results).toBeHidden()
  await expect
    .poll(async () => (await page.evaluate(() => window.kchess.loadData())).settings.soundEnabled)
    .toBe(!before)

  // Unprefixed text searches everything, with acronym matches.
  await page.keyboard.press('ControlOrMeta+K')
  await input.fill('otb')
  await expect(results.getByRole('option').first()).toContainText('Over the board')
  await input.fill('stockfish black')
  await expect(results.getByRole('option').first()).toContainText('Play Stockfish')
  await expect(results.getByRole('option').first()).toContainText('Black')
  await testInfo.attach('quick-pick-search', {
    body: await page.screenshot(),
    contentType: 'image/png',
  })
  await page.keyboard.press('Escape')
  await expect(results).toBeHidden()

  // The top bar's own box expands in place; clicking anywhere else, even on the top bar, closes it.
  const closedBox = await input.boundingBox()
  await input.click()
  await expect(input).toBeFocused()
  await expect(results).toBeVisible()
  expect(await input.boundingBox()).toEqual(closedBox)
  await page.locator('.main-content').click({ position: { x: 20, y: 600 } })
  await expect(results).toBeHidden()
  await expect(input).not.toBeFocused()
  await input.click()
  await expect(results).toBeVisible()
  // Click bare topbar chrome: the far right holds the frameless window
  // controls on Windows/Linux (Close would kill the window) and the empty
  // left end collapses to zero height, so click the header's padded left
  // edge, which has no single-click action.
  const topbar = (await page.locator('.topbar').boundingBox())!
  await page.locator('.topbar').click({ position: { x: 10, y: topbar.height / 2 } })
  await expect(results).toBeHidden()
})

test('keeps multi-chapter studies together and restores the selected chapter offline', async ({
  desktop: { page },
}) => {
  await page.evaluate(() => {
    localStorage.setItem(
      'kchess:studies:v1',
      JSON.stringify({
        version: 2,
        items: [
          {
            id: 'offline-study',
            name: 'Offline repertoire',
            updatedAt: Date.now(),
            pgn: '1. e4 e5 *',
            chapters: [
              { id: 'king', name: 'King pawn', pgn: '1. e4 e5 *' },
              { id: 'queen', name: 'Queen pawn', pgn: '1. d4 {Offline notes} d5 (1... Nf6 $1) *' },
            ],
          },
        ],
      }),
    )
    localStorage.setItem(
      'kchess:analysis:v1',
      JSON.stringify({ version: 1, pgn: '1. c4 *', path: '', orientation: 'white', study: '' }),
    )
  })
  await page.reload()
  await navigate(page, 'Studies')
  await expect(page.locator('.study-card')).toHaveCount(1)
  await page.getByRole('button', { name: 'Open Offline repertoire', exact: true }).click()
  await expect(page.getByRole('dialog', { name: 'Replace the unsaved analysis?' })).toBeVisible()
  await page.getByRole('button', { name: 'Replace', exact: true }).click()
  await page.getByRole('button', { name: 'Study chapter' }).click()
  await page.getByRole('option', { name: '2. Queen pawn', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Study chapter' })).toContainText('Queen pawn')
  await expect(page.getByLabel('Study name', { exact: true })).toHaveValue('Offline repertoire')
  await page.reload()
  await expect(page.getByRole('button', { name: 'Study chapter' })).toContainText('Queen pawn')
  await expect
    .poll(() =>
      page.evaluate(() => JSON.parse(localStorage.getItem('kchess:analysis:v1')!).chapter),
    )
    .toBe('queen')
  await expect(page.getByRole('textbox', { name: 'Chapter name', exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: 'Chapter actions', exact: true }).click()
  await page.getByRole('menuitem', { name: 'Duplicate chapter', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Study chapter' })).toContainText(
    'Queen pawn (copy)',
  )
  await page.getByRole('button', { name: 'Chapter actions', exact: true }).click()
  await page.getByRole('menuitem', { name: 'Delete chapter', exact: true }).click()
  await expect(page.getByRole('dialog', { name: 'Delete “Queen pawn (copy)”?' })).toBeVisible()
  await page.getByRole('button', { name: 'Delete chapter', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Study chapter' })).toContainText('Queen pawn')
  await expect
    .poll(() =>
      page.evaluate(
        () => JSON.parse(localStorage.getItem('kchess:studies:v1')!).items[0].chapters.length,
      ),
    )
    .toBe(2)
  await page.screenshot({ path: test.info().outputPath('offline-study-chapters.png') })
  await page.getByRole('button', { name: 'Study actions', exact: true }).click()
  await page.getByRole('menuitem', { name: 'Close study', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Study chapter', exact: true })).toHaveCount(0)
  await expect(page.getByText('Unsaved analysis', { exact: true })).toBeVisible()
  await expect(
    page.getByRole('group', { name: 'Moves', exact: true }).locator('.tree-move'),
  ).toHaveCount(0)
  await expect(page.locator('.moves-empty')).toBeVisible()
})

test('analyses a broadcast inline and explores lines without changing saved analysis', async ({
  desktop: { app, page },
}, testInfo) => {
  await app.evaluate(() => {
    globalThis.fetch = async (input, init) => {
      const path = new URL(new Request(input, init).url).pathname
      const tour = { id: 'Broad001', name: 'Training broadcast' }
      const round = { id: 'Round001', name: 'Round 1', ongoing: true }
      if (path === '/api/broadcast/top') return Response.json({ active: [{ tour, round }] })
      if (path === '/api/broadcast/Broad001')
        return Response.json({ tour, rounds: [round], defaultRoundId: round.id })
      if (path === '/api/stream/broadcast/round/Round001.pgn')
        return new Response(
          new ReadableStream({
            start(controller) {
              controller.enqueue(
                new TextEncoder().encode(
                  '[Event "Training broadcast"]\n[White "Alpha"]\n[Black "Beta"]\n[GameURL "https://lichess.org/broadcast/training/round-1/Round001/Board001"]\n[Result "*"]\n\n1. e4 {[%clk 0:01:00]} e5 {[%clk 0:01:10]} *\n\n\n',
                ),
              )
            },
          }),
          { headers: { 'Content-Type': 'application/x-chess-pgn' } },
        )
      if (path === '/api/tv/channels') return Response.json({})
      throw new Error('Unexpected fixture request: ' + path)
    }
  })
  await navigate(page, 'Watch')
  await page.getByRole('tab', { name: 'Broadcasts', exact: true }).click()
  // Failed verification without a game ID is uncertainty, not an active game.
  await app.evaluate(({ BrowserWindow }) =>
    BrowserWindow.getAllWindows()[0]!.webContents.send('online:state', {
      session: 99,
      lane: 'game',
      phase: 'disconnected',
      gameId: 'OldGame1',
    }),
  )
  await expect(page.getByText('You are in a game', { exact: true })).toBeVisible()
  await app.evaluate(({ BrowserWindow }) =>
    BrowserWindow.getAllWindows()[0]!.webContents.send('online:state', {
      session: 100,
      lane: 'game',
      phase: 'disconnected',
      gameId: '',
      message: 'Game status request failed',
    }),
  )
  await expect(page.getByText('You are in a game', { exact: true })).toHaveCount(0)
  await expect(page.getByText('Game status not verified', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: /Training broadcast/ })).toBeEnabled()
  await expect(page.getByRole('button', { name: 'Check again', exact: true })).toBeEnabled()
  await app.evaluate(({ BrowserWindow }) =>
    BrowserWindow.getAllWindows()[0]!.webContents.send('online:state', {
      session: 100,
      lane: 'game',
      phase: 'idle',
      gameId: '',
    }),
  )
  await expect(page.getByText('Game status not verified', { exact: true })).toHaveCount(0)
  const headingGap = await page
    .getByRole('heading', { name: 'Live now', exact: true })
    .evaluate(
      (heading) =>
        heading.nextElementSibling!.getBoundingClientRect().top -
        heading.getBoundingClientRect().bottom,
    )
  expect(headingGap).toBe(12)
  await page.getByRole('button', { name: /Training broadcast/ }).click()
  const gridBar = page.locator('.broadcast-grid').getByRole('meter', { name: 'Evaluation' })
  await expect(gridBar).toHaveCount(1)
  await expect(gridBar).toHaveAttribute('aria-valuenow', /^\d+$/)
  const gridSwitch = page.getByRole('switch', { name: 'Evaluation bars', exact: true })
  await gridSwitch.click()
  await expect(gridBar).toHaveCount(0)
  await gridSwitch.click()
  await expect(gridBar).toHaveAttribute('aria-valuenow', /^\d+$/)
  await page.screenshot({ path: testInfo.outputPath('broadcast-grid-evaluations.png') })
  await page.getByRole('button', { name: /Beta.*Alpha/ }).click()
  const whiteClock = page.getByLabel('white clock', { exact: true })
  await expect(page.locator('.player-line.active .player-clock')).toHaveAttribute(
    'aria-label',
    'white clock',
  )
  const beforeTick = await whiteClock.innerText()
  await expect(whiteClock).not.toHaveText(beforeTick)
  await expect(page.getByLabel('black clock', { exact: true })).toHaveText('1:10')
  const toggle = page.getByRole('switch', { name: 'Broadcast engine analysis', exact: true })
  await toggle.click()
  await expect(page.getByRole('meter', { name: 'Evaluation' })).toBeVisible()
  const broadcastBoardWidth = (await page.locator('.broadcast-board .cg-wrap').boundingBox())!.width
  const broadcastPanelWidth = (await page.locator('.broadcast-layout .side-panel').boundingBox())!
    .width
  const lines = page.getByRole('list', { name: 'Engine lines' })
  await expect(lines.locator('li')).toHaveCount(3)
  await page.getByRole('button', { name: 'Move 1, white, e4', exact: true }).click()
  // Historical board navigation does not change which live clock is running.
  await expect(page.locator('.player-line.active .player-clock')).toHaveAttribute(
    'aria-label',
    'white clock',
  )
  await expect(lines.locator('.pv-move').first()).toHaveText(/^1… /)
  await page.getByRole('button', { name: 'Go to first move', exact: true }).click()
  await expect(lines.locator('.pv-move').first()).toHaveText(/^1\. /)
  await page.getByRole('button', { name: 'Go to last move', exact: true }).click()
  await expect(lines.locator('.pv-move').first()).toHaveText(/^2\. /)
  const move = lines.locator('.pv-move').first()
  await move.hover()
  await expect(page.locator('.pv-preview .cg-wrap')).toBeVisible()
  expect(
    await lines
      .locator('.pv-row')
      .first()
      .evaluate((row) => row.getBoundingClientRect().height),
  ).toBeLessThanOrEqual(36)
  await move.click()
  await expect(page.locator('.pv-preview')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Return to game', exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Return to game', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Return to game', exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: 'Broadcast analysis settings', exact: true }).click()
  await expect(page.getByRole('switch', { name: 'Best move arrows', exact: true })).toBeVisible()
  await page.getByRole('switch', { name: 'Evaluation bar', exact: true }).click()
  await expect(page.getByRole('meter', { name: 'Evaluation' })).toHaveCount(0)
  expect((await page.locator('.broadcast-board .cg-wrap').boundingBox())!.width).toBe(
    broadcastBoardWidth,
  )
  await page.keyboard.press('Escape')
  await toggle.click()
  await expect(lines).toHaveCount(0)
  await navigate(page, 'Analysis board')
  expect((await page.locator('.analysis-board .cg-wrap').boundingBox())!.width).toBe(
    broadcastBoardWidth,
  )
  expect((await page.locator('.analysis-panel').boundingBox())!.width).toBe(broadcastPanelWidth)
  await expect(page.getByText('Unsaved analysis', { exact: true })).toBeVisible()
  await expect(
    page.getByRole('group', { name: 'Moves', exact: true }).locator('.tree-move'),
  ).toHaveCount(0)
})

test('practices every trainer offline without leaving the page', async ({ desktop: { page } }) => {
  await navigate(page, 'Practice')
  await page.getByRole('tab', { name: 'Coordinates', exact: true }).click()
  await expect(page.getByText('Board from the side of', { exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Square colours', exact: true }).click()
  await expect(page.getByText(/Name each square/)).toBeVisible()
  await page.getByRole('tab', { name: 'Knight paths', exact: true }).click()
  await expect(page.getByText('Reach the green square in the fewest moves.')).toBeVisible()
  await page.getByRole('tab', { name: 'Endgames', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Basic mates', exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Openings', exact: true }).click()
  await expect(page.getByText('Save a repertoire first', { exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Your mistakes', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Analyze a game', exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Puzzle themes', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Mate patterns', exact: true })).toBeVisible()
})

test('scores coordinates and square-colour runs from board and keyboard', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Practice')
  await page.getByRole('tab', { name: 'Coordinates', exact: true }).click()
  await page.getByRole('button', { name: 'Start', exact: true }).click()
  const target = page.locator('.coord-target')
  await expect(target).toBeVisible()
  const square = (await target.innerText()).trim()
  expect(square).toMatch(/^[a-h][1-8]$/)
  await clickSquare(page, square)
  await expect(page.locator('.run-score strong')).toHaveText('1')
  await page.getByRole('button', { name: 'End run', exact: true }).click()
  await expect(page.getByText(/accuracy/)).toBeVisible()

  await page.getByRole('tab', { name: 'Square colours', exact: true }).click()
  await page.getByRole('button', { name: 'Start', exact: true }).click()
  const shown = page.locator('.drill-square')
  await expect(shown).toBeVisible()
  const name = (await shown.innerText()).trim()
  expect(name).toMatch(/^[a-h][1-8]$/)
  const file = name.charCodeAt(0) - 97
  const rank = Number(name[1]) - 1
  await page.keyboard.press((file + rank) % 2 === 0 ? 'd' : 'l')
  await expect(page.locator('.drill-stage')).toContainText('1 correct')
})

test('starts a knight run and skips a round', async ({ desktop: { page } }) => {
  await navigate(page, 'Practice')
  await page.getByRole('tab', { name: 'Knight paths', exact: true }).click()
  await page.getByRole('button', { name: 'Start', exact: true }).click()
  await expect(page.locator('.knight-target')).toContainText(/^Reach [a-h][1-8] in \d+ jump/)
  await page.getByRole('button', { name: 'Skip', exact: true }).click()
  await expect(page.getByText(/Skipped — the shortest path was/)).toBeVisible()
  await page.getByRole('button', { name: 'End run', exact: true }).click()
  await expect(page.getByText(/shortest paths/)).toBeVisible()
})

test('browses puzzle tabs and shows offline account states', async ({ desktop: { app, page } }) => {
  await app.evaluate(() => {
    globalThis.fetch = () => Promise.reject(new Error('Offline puzzles test.'))
  })
  await navigate(page, 'Puzzles')
  await page.getByRole('tab', { name: 'Daily', exact: true }).click()
  await expect(page.getByText('Couldn’t get the daily puzzle', { exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Storm · Streak · Rush', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Play a run', exact: true })).toBeVisible()
  await expect(page.getByRole('radio', { name: /Storm/ })).toBeVisible()
  await page.getByRole('tab', { name: 'Lichess stats', exact: true }).click()
  await expect(page.getByText('Connect a Lichess account', { exact: true }).first()).toBeVisible()
  await page.getByRole('tab', { name: 'Lichess history', exact: true }).click()
  await expect(page.getByText('Connect a Lichess account', { exact: true }).first()).toBeVisible()
})

test('validates account-free online, following and player lookup', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Play on Lichess')
  await expect(page.getByText('Connect Lichess to play', { exact: true })).toBeVisible()
  await navigate(page, 'Players')
  await page.getByRole('textbox', { name: 'Lichess username', exact: true }).fill('!')
  await page.getByRole('button', { name: 'Look up', exact: true }).click()
  await expect(page.getByText('Enter a Lichess username.', { exact: true })).toBeVisible()
  await page.evaluate(() => {
    Object.defineProperty(navigator, 'onLine', { configurable: true, get: () => false })
    window.dispatchEvent(new Event('offline'))
  })
  await expect(
    page.getByText('Offline · reconnect to look up players.', { exact: true }),
  ).toBeVisible()
  await expect(page.getByRole('button', { name: 'Look up', exact: true })).toBeDisabled()
  await navigate(page, 'Following')
  await expect(page.getByText('Follow your first player', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Follow player', exact: true })).toBeDisabled()
  await navigate(page, 'Play on Lichess')
  await expect(
    page.getByText('Offline · reconnect to play on Lichess.', { exact: true }),
  ).toBeVisible()
})

test('watches TV channels and followed players without an account', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Watch')
  await expect(page.getByRole('heading', { name: 'Channels', exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Following', exact: true }).click()
  await expect(page.getByText('Followed players playing now', { exact: true })).toBeVisible()
  await expect(page.getByText('No followed player is playing right now.')).toBeVisible()
  await page.getByRole('tab', { name: 'TV', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Channels', exact: true })).toBeVisible()
})

test('filters tournaments and guards arena creation offline', async ({
  desktop: { app, page },
}) => {
  await app.evaluate(() => {
    const original = globalThis.fetch
    globalThis.fetch = async (input, init) => {
      const request = new Request(input, init)
      if (new URL(request.url).pathname.startsWith('/api/tournament'))
        return Response.json({ started: [], created: [] })
      return original(input, init)
    }
  })
  await navigate(page, 'Tournaments')
  await page.getByRole('tab', { name: 'Playable here', exact: true }).click()
  await expect(page.getByRole('tab', { name: 'All', exact: true })).toBeVisible()
  await page.evaluate(() => {
    Object.defineProperty(navigator, 'onLine', { configurable: true, get: () => false })
    window.dispatchEvent(new Event('offline'))
  })
  await expect(
    page.getByText('Offline · standings may be out of date.', { exact: true }),
  ).toBeVisible()
  await expect(page.getByRole('button', { name: 'Create arena', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeDisabled()
})

test('guards local game creation and runs the chess clock', async ({ desktop: { page } }) => {
  await navigate(page, 'Over the board')
  await expect(page.getByRole('button', { name: 'Take back', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: 'New game', exact: true }).click()
  await expect(page.getByRole('dialog', { name: 'New game over the board' })).toBeVisible()
  await page.getByPlaceholder('Standard start').fill('not-a-fen')
  await expect(page.getByRole('button', { name: 'Start', exact: true })).toBeDisabled()
  await page.keyboard.press('Escape')
  await page.getByRole('tab', { name: 'Chess clock', exact: true }).click()
  await expect(page.getByText('Layout', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Start', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Pause', exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Reset', exact: true }).click()
})

test('persists gameplay and sound settings', async ({ desktop: { page } }) => {
  const before = await page.evaluate(async () => (await window.kchess.loadData()).settings)
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+,' : 'Control+,')
  await navigate(page, 'Gameplay')
  const moves = page.getByRole('switch', { name: 'Show possible moves', exact: true })
  await expect(moves).toBeVisible()
  await moves.click()
  await expect
    .poll(async () => (await page.evaluate(() => window.kchess.loadData())).settings.showLegalMoves)
    .toBe(!before.showLegalMoves)
  await navigate(page, 'Sound')
  const sound = page.getByRole('switch', { name: 'Game sounds', exact: true })
  await expect(sound).toBeVisible()
  await sound.click()
  await expect
    .poll(async () => (await page.evaluate(() => window.kchess.loadData())).settings.soundEnabled)
    .toBe(!before.soundEnabled)
})

test('validates study import, editor FEN and analysis import', async ({ desktop: { page } }) => {
  await navigate(page, 'Studies')
  await expect(page.getByText('No studies yet', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Import PGN', exact: true }).first().click()
  await page.getByLabel('PGN', { exact: true }).fill('[FEN "not-a-fen"]\n\n1. e4 e5 *')
  await page.getByRole('dialog').getByRole('button', { name: 'Import', exact: true }).click()
  await expect(page.getByText('Paste valid PGN with up to 64 chapters.')).toBeVisible()
  await page.keyboard.press('Escape')

  await navigate(page, 'Board editor')
  await page.getByLabel('FEN', { exact: true }).fill('bad-fen')
  await page.keyboard.press('Enter')
  await expect(page.getByText('That is not a valid FEN.', { exact: true })).toBeVisible()

  await navigate(page, 'Analysis board')
  await page.locator('.analysis-panel').getByRole('button', { name: 'Import', exact: true }).click()
  await page.getByRole('textbox', { name: 'FEN or PGN' }).fill('[FEN "not-a-fen"]\n\n1. e4 e5 *')
  await page.getByRole('dialog').getByRole('button', { name: 'Import', exact: true }).click()
  await expect(
    page.getByText('Use a valid FEN or one PGN game with legal moves, under 2 MB.'),
  ).toBeVisible()
})
