// Builds app/assets/openings.json from Lichess's CC0 opening names (github.com/lichess-org/chess-openings).
// Usage: node scripts/make-openings.mjs [dir-with-a..e.tsv]  (downloads the TSVs when no dir is given)
import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { makeFen } from 'chessops/fen'
import { parsePgn, startingPosition } from 'chessops/pgn'
import { parseSan } from 'chessops/san'

const SOURCE = 'https://raw.githubusercontent.com/lichess-org/chess-openings/master'
const dir = process.argv[2]
const entries = {}
for (const volume of ['a', 'b', 'c', 'd', 'e']) {
  const text = dir
    ? await readFile(join(dir, `${volume}.tsv`), 'utf8')
    : await (await fetch(`${SOURCE}/${volume}.tsv`)).text()
  for (const line of text.split('\n').slice(1)) {
    const [eco, name, pgn] = line.split('\t')
    if (!eco || !name || !pgn) continue
    const game = parsePgn(pgn)[0]
    if (!game) continue
    const pos = startingPosition(game.headers).unwrap()
    let ok = true
    for (const node of game.moves.mainline()) {
      const move = parseSan(pos, node.san)
      if (!move) {
        ok = false
        break
      }
      pos.play(move)
    }
    if (!ok) throw new Error(`Illegal line for ${name}: ${pgn}`)
    // EPD: placement, side, castling, en passant (chessops writes it only when legal).
    const epd = makeFen(pos.toSetup()).split(' ').slice(0, 4).join(' ')
    // Several lines may name one position; the shortest line's name wins, like Lichess.
    const plies = pgn.split(/\s+/).filter((token) => !/^\d+\.$/.test(token)).length
    const known = entries[epd]
    if (!known || plies < known[2]) entries[epd] = [eco, name, plies]
  }
}
const compact = Object.fromEntries(
  Object.entries(entries)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([epd, [eco, name]]) => [epd, `${eco}|${name}`]),
)
await writeFile(
  new URL('../app/assets/openings.json', import.meta.url),
  JSON.stringify(compact) + '\n',
)
console.log(`${Object.keys(compact).length} named positions`)
