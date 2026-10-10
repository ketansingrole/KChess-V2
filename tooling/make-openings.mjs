// Builds crates/kchess-wasm/js/data/openings.json from Lichess's CC0 opening names (github.com/lichess-org/chess-openings).
// Usage: node tooling/make-openings.mjs [dir-with-a..e.tsv]  (downloads the TSVs when no dir is given)
import { createRequire } from 'node:module'
import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

const SOURCE = 'https://raw.githubusercontent.com/lichess-org/chess-openings/master'
const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
// The Rust rules (`pgnMainline`), loaded as `tooling/native-rules.ts` loads them.
const native = createRequire(new URL('../crates/kchess-node/js/native.ts', import.meta.url))(
  '@kchess/native/binding',
)
const dir = process.argv[2]
const entries = {}
for (const volume of ['a', 'b', 'c', 'd', 'e']) {
  const text = dir
    ? await readFile(join(dir, `${volume}.tsv`), 'utf8')
    : await (await fetch(`${SOURCE}/${volume}.tsv`)).text()
  for (const line of text.split('\n').slice(1)) {
    const [eco, name, pgn] = line.split('\t')
    if (!eco || !name || !pgn) continue
    const main = JSON.parse(native.invoke('pgnMainline', JSON.stringify([pgn])))
    if (!main) continue
    const plies = pgn.split(/\s+/).filter((token) => !/^\d+\.$/.test(token)).length
    // Played until the first illegal move, which the line must not have.
    if (main.moves.length !== plies) throw new Error(`Illegal line for ${name}: ${pgn}`)
    const fen = main.moves.at(-1)?.fen ?? main.start ?? START
    // EPD: placement, side, castling, en passant (the FEN writes it only when legal).
    const epd = fen.split(' ').slice(0, 4).join(' ')
    // Several lines may name one position; the shortest line's name wins, like Lichess.
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
  new URL('../crates/kchess-wasm/js/data/openings.json', import.meta.url),
  JSON.stringify(compact) + '\n',
)
console.log(`${Object.keys(compact).length} named positions`)
