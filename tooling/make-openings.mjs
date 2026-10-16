// Builds crates/kchess-domain/data/openings.json from Lichess's CC0 opening names (github.com/lichess-org/chess-openings).
// Usage: node tooling/make-openings.mjs [dir-with-a..e.tsv]  (downloads the TSVs when no dir is given)
//
// Every named line is kept as `[eco, name, uciMoves]`, in the source's order (volumes a–e, row by
// row). The Rust rules (`crates/kchess-domain/src/training/openings.rs`) derive the rest from it: the
// name of each position (the shortest line naming it wins, the first on a tie, like Lichess) and the
// family tree (`Family: Variation, Subvariation`).
import { createRequire } from 'node:module'
import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

const SOURCE = 'https://raw.githubusercontent.com/lichess-org/chess-openings/master'
// The Rust rules (`pgnMainline`), loaded as `tooling/native-rules.ts` loads them.
const native = createRequire(new URL('../crates/kchess-node/js/native.ts', import.meta.url))(
  '@kchess/native/binding',
)
const dir = process.argv[2]
const lines = []
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
    lines.push([eco, name, main.moves.map((move) => move.uci).join(' ')])
  }
}
await writeFile(
  new URL('../crates/kchess-domain/data/openings.json', import.meta.url),
  JSON.stringify({ lines }) + '\n',
)
console.log(`${lines.length} named lines`)
