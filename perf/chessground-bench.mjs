// Chessground update cost: a full `set(configuration())` versus `setShapes` for arrow-only
// changes, the shape the ChessBoard watcher had before it was split.
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { chromium } from '@playwright/test'

const root = resolve(import.meta.dirname, '..')
const server = createServer(async (request, response) => {
  const path = new URL(request.url ?? '/', 'http://x').pathname
  if (path === '/' || path === '/index.html') {
    response.setHeader('content-type', 'text/html')
    response.end(`<!doctype html><meta charset="utf-8">
<style>#board{width:400px;height:400px;position:relative}</style>
<div id="board"></div>
<script type="module">
  import { Chessground } from '/node_modules/@lichess-org/chessground/dist/chessground.js'
  const dests = new Map([['e2',['e3','e4']],['d2',['d3','d4']],['g1',['f3','h3']],['b1',['a3','c3']]])
  const config = () => ({
    fen: 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1',
    orientation: 'white',
    turnColor: 'white',
    check: false,
    lastMove: ['e2','e4'],
    coordinates: true,
    highlight: { lastMove: true, check: true },
    animation: { enabled: true, duration: 200 },
    movable: { free: false, color: 'both', dests, showDests: true, rookCastle: true, events: {} },
    premovable: { enabled: false, showDests: true, castle: true },
    draggable: { enabled: true, showGhost: true },
    selectable: { enabled: true },
    drawable: { enabled: true, visible: true, autoShapes: [{ orig: 'e2', dest: 'e4', brush: 'green' }] },
    events: {},
  })
  const ground = Chessground(document.getElementById('board'), config())
  const shapes = Array.from({ length: 6 }, (_, i) => ({ orig: 'e2', dest: 'e4', brush: 'green', label: String(i) }))
  function bench(fn, n) {
    const start = performance.now()
    for (let i = 0; i < n; i++) fn(i)
    return performance.now() - start
  }
  window.benchSet = (n) => bench(() => ground.set(config()), n)
  window.benchShapes = (n) => bench((i) => ground.setShapes(shapes.map((s) => ({ ...s, label: String(i) }))), n)
  window.ready = true
</script>`)
    return
  }
  try {
    const body = await readFile(join(root, path))
    const type = path.endsWith('.js')
      ? 'text/javascript'
      : path.endsWith('.css')
        ? 'text/css'
        : 'application/octet-stream'
    response.setHeader('content-type', type)
    response.end(body)
  } catch {
    response.statusCode = 404
    response.end('not found')
  }
})
await new Promise((done) => server.listen(0, '127.0.0.1', done))
const { port } = server.address()
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH })
const page = await browser.newPage()
await page.goto(`http://127.0.0.1:${port}/`)
await page.waitForFunction(() => window.ready)
const rounds = 7
const results = { setConfig: [], setShapes: [] }
for (let i = 0; i < rounds; i++) {
  results.setConfig.push(await page.evaluate(() => window.benchSet(200)))
  results.setShapes.push(await page.evaluate(() => window.benchShapes(200)))
}
await browser.close()
server.close()
const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)]
const summary = {
  setConfigMsPer200: median(results.setConfig),
  setShapesMsPer200: median(results.setShapes),
  setConfigMsEach: median(results.setConfig) / 200,
  setShapesMsEach: median(results.setShapes) / 200,
  samples: results,
}
console.log(JSON.stringify(summary, null, 2))
