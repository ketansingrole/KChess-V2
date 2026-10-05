import { parseUci } from 'chessops/util'
import { normalizeMove, type Position } from 'chessops/chess'
import { replaySetup, type GameSetup } from '../../src/shared/variant'
import { encodeGif, type GifFrame } from './gif'
import { pieceUrl } from './pieces'

const boards = import.meta.glob<string>('../assets/boards/*.{png,jpg}', {
  eager: true,
  query: '?url',
  import: 'default',
})
function boardUrl(theme: string): string {
  const match = Object.entries(boards).find(([path]) => path.includes(`/boards/${theme}.`))
  return (match ?? Object.entries(boards).find(([path]) => path.includes('/boards/brown.'))!)[1]
}

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image()
    image.onload = () => resolve(image)
    image.onerror = () => reject(new Error('An image for the export could not be loaded.'))
    image.src = url
  })
}

export interface ImageOptions {
  setup: GameSetup
  moves: readonly string[]
  orientation: 'white' | 'black'
  boardTheme: string
  pieceSet: string
  white: string
  black: string
  /** Board width in pixels. */
  size?: number
}

const ROLE_LETTER = {
  pawn: 'P',
  knight: 'N',
  bishop: 'B',
  rook: 'R',
  queen: 'Q',
  king: 'K',
} as const
const BAR = 28

interface Painter {
  canvas: HTMLCanvasElement
  context: CanvasRenderingContext2D
  draw(pos: Position, lastMove?: string): void
}

async function painter(options: ImageOptions): Promise<Painter> {
  const size = Math.round((options.size ?? 360) / 8) * 8
  const square = size / 8
  const canvas = document.createElement('canvas')
  canvas.width = size
  canvas.height = size + 2 * BAR
  const context = canvas.getContext('2d', { willReadFrequently: true })
  if (!context) throw new Error('Drawing is not available here.')
  const board = await loadImage(boardUrl(options.boardTheme))
  const pieces = new Map<string, HTMLImageElement>()
  await Promise.all(
    (['w', 'b'] as const).flatMap((color) =>
      (['P', 'N', 'B', 'R', 'Q', 'K'] as const).map(async (role) =>
        pieces.set(`${color}${role}`, await loadImage(pieceUrl(options.pieceSet, color, role))),
      ),
    ),
  )
  const flipped = options.orientation === 'black'
  const top = flipped ? options.white : options.black
  const bottom = flipped ? options.black : options.white
  const at = (sq: number): [number, number] => {
    const file = sq % 8,
      rank = Math.floor(sq / 8)
    return flipped
      ? [(7 - file) * square, rank * square + BAR]
      : [file * square, (7 - rank) * square + BAR]
  }
  return {
    canvas,
    context,
    draw(pos, lastMove) {
      context.fillStyle = '#262421'
      context.fillRect(0, 0, canvas.width, canvas.height)
      context.fillStyle = '#e8e6e3'
      context.font = `600 ${Math.round(BAR * 0.5)}px system-ui, sans-serif`
      context.textBaseline = 'middle'
      context.fillText(top, 8, BAR / 2, size - 16)
      context.fillText(bottom, 8, size + BAR + BAR / 2, size - 16)
      context.drawImage(board, 0, BAR, size, size)
      const move = lastMove ? parseUci(lastMove) : undefined
      if (move && 'from' in move) {
        context.fillStyle = 'rgba(155, 199, 0, 0.41)'
        for (const sq of [move.from, move.to]) {
          const [x, y] = at(sq)
          context.fillRect(x, y, square, square)
        }
      }
      for (const [sq, piece] of pos.board) {
        const image = pieces.get(`${piece.color === 'white' ? 'w' : 'b'}${ROLE_LETTER[piece.role]}`)
        const [x, y] = at(sq)
        if (image) context.drawImage(image, x, y, square, square)
      }
    },
  }
}

/** Each position of the game, from the start; `upTo` stops after that many moves. */
function positions(
  options: ImageOptions,
  upTo = options.moves.length,
): { pos: Position; move?: string }[] {
  const replayed = replaySetup(options.setup, options.moves.slice(0, upTo))
  if (!replayed) throw new Error('That game cannot be replayed.')
  const pos = replayed.start.clone()
  const list: { pos: Position; move?: string }[] = [{ pos: pos.clone() }]
  for (const move of replayed.played) {
    const parsed = parseUci(move.uci)
    if (!parsed) break
    pos.play(normalizeMove(pos, parsed))
    list.push({ pos: pos.clone(), move: move.uci })
  }
  return list
}

/** A PNG of the position after `ply` moves. */
export async function positionPng(options: ImageOptions, ply: number): Promise<Uint8Array> {
  const paint = await painter(options)
  const shown = positions(options, ply).at(-1)!
  paint.draw(shown.pos, shown.move)
  const blob = await new Promise<Blob | null>((resolve) =>
    paint.canvas.toBlob(resolve, 'image/png'),
  )
  if (!blob) throw new Error('The image could not be made.')
  return new Uint8Array(await blob.arrayBuffer())
}

/** An animated GIF of the whole game; `delay` is the time per move in hundredths of a second. */
export async function gameGif(options: ImageOptions, delay = 80): Promise<Uint8Array> {
  const paint = await painter(options)
  const frames: GifFrame[] = []
  const list = positions(options)
  if (list.length > 600) throw new Error('That game is too long for a GIF (300 moves at most).')
  list.forEach((entry, index) => {
    paint.draw(entry.pos, entry.move)
    frames.push({
      rgba: paint.context.getImageData(0, 0, paint.canvas.width, paint.canvas.height).data,
      // Linger on the start and on the final position.
      delay: index === 0 ? delay * 2 : index === list.length - 1 ? 300 : delay,
    })
  })
  return encodeGif(paint.canvas.width, paint.canvas.height, frames)
}
