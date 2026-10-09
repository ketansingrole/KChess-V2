import { scopedState } from './platform'
import { analysisContext } from '../domain/analysisContext'
import { assertAnalysisRequest } from '../domain/validate'
import type { AnalysisRequest, AnalysisUpdate, EngineLine } from '../contracts/types'
import { parseInfo } from '../domain/uciInfo'
import { engineIdentity, engineStatus, spawnEngine } from './engine'
import { UciController, SearchCancelled, ensureEngineOptions } from './uci'
import { withEngineLease, searchThreads } from './engineScheduler'
import { errorSummary, logDebug, logWarn, truncateForLog } from './logger'

const DEPTH_LIMIT = 26
const TIME_LIMIT_MS = 60_000
const IDLE_MS = 120_000

export async function startAnalysis(
  raw: unknown,
  configured: string,
  send: (update: AnalysisUpdate) => void,
): Promise<number> {
  const request: AnalysisRequest = assertAnalysisRequest(raw)
  const id = ++serviceState.nextId
  serviceState.current?.abort()
  const controller = new AbortController()
  serviceState.current = controller
  clearTimeout(serviceState.idleTimer)
  serviceState.serial = serviceState.serial
    .catch((error: unknown) => {
      logDebug('analysis', 'Previous analysis failed:', error)
    })
    .then(async () => {
      const lines = new Map<number, EngineLine>()
      let nps: number | undefined
      let key = ''
      let lastEmit = 0
      const context = analysisContext(request.rootFen ?? request.fen, request.moves)
      const whiteToMove = (request.fen.split(' ')[1] ?? 'w') === 'w'
      const emit = (reason?: AnalysisUpdate['reason'], error?: string): void =>
        send({
          id,
          fen: request.fen,
          context,
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
        if (serviceState.engine?.key !== key || serviceState.engine.uci.failed) {
          serviceState.engine?.uci.close()
          serviceState.engine = { key, uci: new UciController(spawnEngine(status)) }
        }
        const target = serviceState.engine.uci
        await withEngineLease(
          2,
          () => controller.abort(),
          controller.signal,
          async () => {
            await target.ready
            controller.signal.throwIfAborted()
            // Stepping through a game reuses this process with identical options;
            // only changed values are sent, so scrubbing skips redundant round-trips
            // (and redundant Hash clears that wipe the transposition table).
            // Chessops stores castling as king-to-rook UCI, including standard games.
            // Without UCI_Chess960 Stockfish stops replaying history at the first castle.
            await ensureEngineOptions(target, {
              Threads: String(searchThreads()),
              Hash: '128',
              MultiPV: String(request.lines),
              UCI_Chess960: 'true',
            })
            controller.signal.throwIfAborted()
            target.write(
              `position fen ${request.rootFen ?? request.fen}${request.moves?.length ? ` moves ${request.moves.join(' ')}` : ''}`,
            )
            await target.search(
              request.infinite
                ? 'go infinite'
                : `go depth ${DEPTH_LIMIT} movetime ${TIME_LIMIT_MS}`,
              (line) => {
                const parsed = parseInfo(line, whiteToMove)
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
          },
        )
      } catch (cause) {
        const context = `fen=${truncateForLog(request.fen, 60)} client=${truncateForLog(request.clientId ?? 'none', 40)}`
        if (controller.signal.aborted || cause instanceof SearchCancelled) {
          logDebug('analysis', 'Analysis interrupted:', context, errorSummary(cause))
          emit('interrupted')
        } else {
          logWarn('analysis', 'Analysis failed:', context, errorSummary(cause))
          emit('failed', cause instanceof Error ? cause.message : String(cause))
        }
      } finally {
        if (serviceState.current === controller) {
          serviceState.current = undefined
          serviceState.idleTimer = setTimeout(() => stopAnalysis(true), IDLE_MS)
          serviceState.idleTimer.unref()
        }
      }
    })
  return id
}
export const analysisRunning = (): boolean => Boolean(serviceState.current)
export function stopAnalysis(kill = false): void {
  serviceState.current?.abort()
  clearTimeout(serviceState.idleTimer)
  if (kill) {
    serviceState.engine?.uci.close()
    serviceState.engine = undefined
  }
}

const serviceState = scopedState(() => ({
  engine: undefined as { key: string; uci: UciController } | undefined,
  current: undefined as AbortController | undefined,
  nextId: 0,
  serial: Promise.resolve(),
  idleTimer: undefined as ReturnType<typeof setTimeout> | undefined,
}))
