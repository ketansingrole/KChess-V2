import { getDb } from './db'
import { logDebug } from './logger'
import type { GameRecord, InsightsQuery, InsightsReport, ReviewSummary } from '../contracts/types'
import { RESULT_SQL } from './store'

const empty = (): GameRecord => ({ total: 0, win: 0, loss: 0, draw: 0 })
function add(record: GameRecord, result: string, count = 1): void {
  record.total += count
  if (result === 'win') record.win += count
  else if (result === 'loss') record.loss += count
  else record.draw += count
}

const RATING_BANDS: [string, number, number][] = [
  ['Much weaker (−200 or less)', -Infinity, -200],
  ['Weaker (−199 to −50)', -199, -50],
  ['Similar (±49)', -49, 49],
  ['Stronger (+50 to +199)', 50, 199],
  ['Much stronger (+200 or more)', 200, Infinity],
]
const LENGTH_BANDS: [string, number, number][] = [
  ['Under 20 moves', 0, 39],
  ['20–39 moves', 40, 79],
  ['40–59 moves', 80, 119],
  ['60 moves or more', 120, Infinity],
]

/**
 * Patterns in the games synced to this computer: results by colour, speed, opening, time of
 * day, opponent strength and game length, and accuracy where games were reviewed. Everything is
 * read locally; nothing is sent to Lichess.
 */
export function insights(query: InsightsQuery): InsightsReport {
  const db = getDb()
  const clauses = ['account = ?']
  const params: (string | number)[] = [query.account]
  if (query.speed) {
    clauses.push('speed = ?')
    params.push(query.speed)
  }
  if (query.rated !== undefined) {
    clauses.push('rated = ?')
    params.push(query.rated ? 1 : 0)
  }
  if (query.days) {
    clauses.push('createdAt >= ?')
    params.push(Date.now() - query.days * 86_400_000)
  }
  // In-progress games have no result yet.
  clauses.push("status NOT IN ('created', 'started')")
  const where = clauses.join(' AND ')
  const rows = db
    .prepare(
      `SELECT id, color, speed, status, opening, opponentRating, playerRating, createdAt,
        (${RESULT_SQL}) AS result,
        CAST(strftime('%w', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS weekday,
        CAST(strftime('%H', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS hour,
        CASE WHEN moves = '' THEN 0 ELSE LENGTH(moves) - LENGTH(REPLACE(moves, ' ', '')) + 1 END AS plies
       FROM games WHERE ${where} ORDER BY createdAt DESC LIMIT 20000`,
    )
    .all(...params) as unknown as {
    id: string
    color: 'white' | 'black'
    speed: string
    status: string
    opening: string | null
    opponentRating: number | null
    playerRating: number | null
    createdAt: number
    result: 'win' | 'loss' | 'draw'
    weekday: number
    hour: number
    plies: number
  }[]

  const report: InsightsReport = {
    account: query.account,
    total: rows.length,
    record: empty(),
    byColor: { white: empty(), black: empty() },
    bySpeed: [],
    byOpening: [],
    byWeekday: Array.from({ length: 7 }, empty),
    byHour: Array.from({ length: 24 }, empty),
    byOpponent: RATING_BANDS.map(([label]) => ({ label, record: empty() })),
    byLength: LENGTH_BANDS.map(([label]) => ({ label, record: empty() })),
    endings: [],
    streaks: { longestWin: 0, longestLoss: 0, current: 0 },
  }
  const speeds = new Map<string, GameRecord>()
  const openings = new Map<string, { record: GameRecord; white: number }>()
  const endings = new Map<string, GameRecord>()
  let run = 0
  let runKind = ''
  // Rows are newest first; streaks are counted oldest first.
  for (let i = rows.length - 1; i >= 0; i--) {
    const row = rows[i]!
    add(report.record, row.result)
    add(report.byColor[row.color], row.result)
    if (!speeds.has(row.speed)) speeds.set(row.speed, empty())
    add(speeds.get(row.speed)!, row.result)
    if (row.opening) {
      // Group variations under their opening family: "Sicilian Defense: Najdorf" → "Sicilian Defense".
      const family = row.opening.split(':')[0]!.trim()
      const entry = openings.get(family) ?? { record: empty(), white: 0 }
      add(entry.record, row.result)
      if (row.color === 'white') entry.white++
      openings.set(family, entry)
    }
    if (row.weekday >= 0 && row.weekday < 7) add(report.byWeekday[row.weekday]!, row.result)
    if (row.hour >= 0 && row.hour < 24) add(report.byHour[row.hour]!, row.result)
    if (row.opponentRating && row.playerRating) {
      const diff = row.opponentRating - row.playerRating
      const band = RATING_BANDS.findIndex(([, low, high]) => diff >= low && diff <= high)
      if (band >= 0) add(report.byOpponent[band]!.record, row.result)
    }
    const length = LENGTH_BANDS.findIndex(([, low, high]) => row.plies >= low && row.plies <= high)
    if (length >= 0) add(report.byLength[length]!.record, row.result)
    if (!endings.has(row.status)) endings.set(row.status, empty())
    add(endings.get(row.status)!, row.result)
    if (row.result === runKind) run++
    else {
      runKind = row.result
      run = 1
    }
    if (runKind === 'win') report.streaks.longestWin = Math.max(report.streaks.longestWin, run)
    if (runKind === 'loss') report.streaks.longestLoss = Math.max(report.streaks.longestLoss, run)
  }
  report.streaks.current = runKind === 'win' ? run : runKind === 'loss' ? -run : 0
  report.bySpeed = [...speeds]
    .map(([speed, record]) => ({ speed, record }))
    .sort((a, b) => b.record.total - a.record.total)
  report.byOpening = [...openings]
    .map(([name, entry]) => ({ name, record: entry.record, asWhite: entry.white }))
    .filter((entry) => entry.record.total >= 2)
    .sort((a, b) => b.record.total - a.record.total)
    .slice(0, 25)
  report.endings = [...endings]
    .map(([status, record]) => ({ status, record }))
    .sort((a, b) => b.record.total - a.record.total)

  // Accuracy of the games that have been reviewed (by Lichess or locally), newest first.
  const reviewed = db
    .prepare(
      `SELECT g.color AS color, r.summary AS summary FROM games g JOIN reviews r ON r.key = (
         SELECT r2.key FROM game_reviews gr JOIN reviews r2 ON r2.key = gr.reviewKey
         WHERE gr.gameId = g.id AND r2.complete = 1 ORDER BY r2.updatedAt DESC LIMIT 1)
       WHERE ${where.replace(/\b(account|speed|rated|createdAt|status)\b/g, 'g.$1')} AND r.complete = 1
       ORDER BY g.createdAt DESC LIMIT 2000`,
    )
    .all(...params) as unknown as { color: 'white' | 'black'; summary: string }[]
  let accuracySum = 0,
    accuracyGames = 0,
    acplSum = 0,
    acplGames = 0
  const mistakes = { inaccuracy: 0, mistake: 0, blunder: 0 }
  for (const row of reviewed) {
    let summary: ReviewSummary
    try {
      summary = JSON.parse(row.summary) as ReviewSummary
    } catch (cause) {
      logDebug('insights', 'Review summary has invalid JSON:', cause)
      continue
    }
    const side = summary[row.color]
    if (!side) continue
    if (typeof side.accuracy === 'number') {
      accuracySum += side.accuracy
      accuracyGames++
    }
    if (typeof side.acpl === 'number') {
      acplSum += side.acpl
      acplGames++
    }
    mistakes.inaccuracy += side.inaccuracy ?? 0
    mistakes.mistake += side.mistake ?? 0
    mistakes.blunder += side.blunder ?? 0
  }
  if (reviewed.length)
    report.accuracy = {
      games: reviewed.length,
      average: accuracyGames ? accuracySum / accuracyGames : undefined,
      acpl: acplGames ? acplSum / acplGames : undefined,
      perGame: {
        inaccuracy: mistakes.inaccuracy / reviewed.length,
        mistake: mistakes.mistake / reviewed.length,
        blunder: mistakes.blunder / reviewed.length,
      },
    }
  return report
}
