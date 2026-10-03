import { analysisContext } from '../shared/analysisContext'
import { assertAnalysisRequest } from '../shared/validate'
import type { AnalysisRequest, AnalysisUpdate, EngineLine } from '../shared/types'
import { parseInfo } from '../shared/uciInfo'
import { engineIdentity, engineStatus, spawnEngine } from './engine'
import { UciController, SearchCancelled } from './uci'
import { acquireEngine, searchThreads } from './engineScheduler'

const DEPTH_LIMIT = 26
const TIME_LIMIT_MS = 60_000
const IDLE_MS = 120_000
let engine: { key: string; uci: UciController } | undefined
let current: AbortController | undefined
let nextId = 0
let serial = Promise.resolve()
let idleTimer: ReturnType<typeof setTimeout> | undefined

export async function startAnalysis(
  raw: unknown,
  configured: string,
  send: (update: AnalysisUpdate) => void,
): Promise<number> {
  const request: AnalysisRequest = assertAnalysisRequest(raw)
  const id = ++nextId
  current?.abort()
  const controller = new AbortController()
  current = controller
  clearTimeout(idleTimer)
  serial = serial
    .catch(() => {})
    .then(async () => {
      const lines = new Map<number, EngineLine>()
      let nps: number | undefined
      let key = ''
      let lastEmit = 0
      let release: (() => void) | undefined
      const emit = (reason?: AnalysisUpdate['reason'], error?: string): void =>
        send({
          id,
          fen: request.fen,
          context: analysisContext(request.rootFen ?? request.fen, request.moves),
          clientId: request.clientId,
          depth: lines.get(1)?.depth ?? 0,
          nps,
          lines: [...lines.values()].sort((a, b) => a.rank - b.rank),
          done: Boolean(reason),
          reason,
          engine: key,
          error,
        })
      try {
        controller.signal.throwIfAborted()
        const status = await engineStatus(configured)
        controller.signal.throwIfAborted()
        if (!status.ready)
          throw new Error('Stockfish could not be found. Choose an engine in Settings.')
        key = await engineIdentity(status)
        controller.signal.throwIfAborted()
        if (engine?.key !== key || engine.uci.failed) {
          engine?.uci.close()
          engine = { key, uci: new UciController(spawnEngine(status)) }
        }
        const target = engine.uci
        release = await acquireEngine(2, () => controller.abort(), controller.signal)
        await target.ready
        controller.signal.throwIfAborted()
        target.write(`setoption name Threads value ${searchThreads()}`)
        target.write('setoption name Hash value 128')
        target.write(`setoption name MultiPV value ${request.lines}`)
        await target.sync()
        controller.signal.throwIfAborted()
        target.write(
          `position fen ${request.rootFen ?? request.fen}${request.moves?.length ? ` moves ${request.moves.join(' ')}` : ''}`,
        )
        await target.search(
          request.infinite ? 'go infinite' : `go depth ${DEPTH_LIMIT} movetime ${TIME_LIMIT_MS}`,
          (line) => {
            const parsed = parseInfo(line, request.fen.split(' ')[1] === 'w')
            if (!parsed || parsed.line.rank > request.lines || controller.signal.aborted) return
            lines.set(parsed.line.rank, parsed.line)
            nps = parsed.nps ?? nps
            if (Date.now() - lastEmit >= 120) {
              lastEmit = Date.now()
              emit()
            }
          },
          request.infinite ? 0 : TIME_LIMIT_MS + 10_000,
          controller.signal,
        )
        emit('completed')
      } catch (cause) {
        if (controller.signal.aborted || cause instanceof SearchCancelled) emit('interrupted')
        else emit('failed', cause instanceof Error ? cause.message : String(cause))
      } finally {
        release?.()
        if (current === controller) {
          current = undefined
          idleTimer = setTimeout(() => stopAnalysis(true), IDLE_MS)
          idleTimer.unref()
        }
      }
    })
  return id
}
export const analysisRunning = (): boolean => Boolean(current)
export function stopAnalysis(kill = false): void {
  current?.abort()
  clearTimeout(idleTimer)
  if (kill) {
    engine?.uci.close()
    engine = undefined
  }
}
