import { GameArchive, archiveState } from '@kchess/core/gameArchive'
import type { ArchivedGame, CoreMethod } from '@kchess/core'
import { createInterface } from 'node:readline/promises'
import { readFile, stat } from 'node:fs/promises'
import { stdin, stdout, stderr } from 'node:process'
import { createNodeCore, type NodeCore, type NodeCoreOptions } from '@kchess/node'
import { CORE_METHODS, assertLevel } from '@kchess/core'
import { ComputerGame, computerState, LocalGame, localState } from '@kchess/core/gameSession'
import { errorSummary, logWarn } from '@kchess/core/logger'

const HELP = `KChess CLI

Usage: pnpm run cli -- [options] <command>

Commands:
  library                         Print the library as JSON
  status                          Print account and engine status
  login                           Connect a Lichess account in your browser
  call <method> '[arguments]'      Invoke any CoreApi method (JSON argument array)
  call <method> --args-file <path> Read arguments from a JSON file
  analyze <fen>                   Stream engine analysis as JSON lines until Ctrl-C
  play [level]                    Play Stockfish with UCI moves
  local                           Play a local game with UCI moves
  methods                         List available CoreApi methods

Options:
  --data-dir <path>                Use a separate profile (default: ~/.kchess/node)
  --browser open|print            Open the login page or print its URL
  --no-credentials                Disable OS credential storage
  --engine <path>                 Use a native Stockfish executable

Interactive game commands: e2e4, undo, resign, new, status, quit.
Local games also accept draw and pause. Sessions are saved in the core library.
`

export function parseCli(argv: string[]): {
  options: NodeCoreOptions
  args: string[]
  engine?: string
} {
  const options: NodeCoreOptions = {}
  const args: string[] = []
  let engine: string | undefined
  for (let i = 0; i < argv.length; i++) {
    const value = argv[i]!
    if (value === '--') continue
    if (value === '--engine') {
      engine = argv[++i]
      if (!engine || engine.startsWith('--')) throw new Error('--engine requires a path.')
    } else if (value === '--data-dir') {
      const path = argv[++i]
      if (!path || path.startsWith('--')) throw new Error('--data-dir requires a path.')
      options.dataDir = path
    } else if (value === '--browser') {
      const browser = argv[++i]
      if (browser !== 'open' && browser !== 'print')
        throw new Error('--browser must be open or print.')
      options.browser = browser
    } else if (value === '--no-credentials') options.disableCredentials = true
    else args.push(value)
  }
  return { options, args, engine }
}
const json = (value: unknown): void => {
  stdout.write(`${JSON.stringify(value ?? null)}\n`)
}

async function interactive(
  core: NodeCore,
  computer: boolean,
  level: string | undefined,
  claimSignals: () => void,
): Promise<void> {
  const library = await core.library()
  let save = Promise.resolve()
  let computerGame: ComputerGame | undefined
  const games = new Map(library.games.map((game) => [game.id, game]))
  const queue = (work: () => Promise<unknown>): void => {
    save = save
      .then(async () => {
        await work()
      })
      .catch((cause) => {
        logWarn('cli', 'Saving failed:', errorSummary(cause))
        process.exitCode = 1
      })
  }
  const persist = (): void => {
    const snapshot = game.snapshot()
    queue(() =>
      computer
        ? core.saveSession('computer', snapshot as ReturnType<ComputerGame['snapshot']>)
        : core.saveSession('local', snapshot as ReturnType<LocalGame['snapshot']>),
    )
    history.save()
    const identity = { ...history.state.identity }
    queue(() => core.saveSession(computer ? 'archive:computer' : 'archive:board', identity))
  }
  const localGame = new LocalGame(localState(library.sessions.local), () => performance.now())
  if (computer) {
    const state = computerState(library.sessions.computer)
    if (level !== undefined) state.level = assertLevel(level)
    // The core still enforces live-game assistance restrictions on every search.
    await core.resumeOnline()
    const engine = await core.engineStatus()
    if (!engine.ready)
      throw new Error('Stockfish is unavailable. Install an engine or configure enginePath.')
    computerGame = new ComputerGame(state, {
      bestMove: (moves, level, options) => core.bestMove(moves, level, options),
      stopEngine: () => core.stopEngine(),
      ready: () => engine.ready,
      allowed: () => true,
      now: () => performance.now(),
      moved(san, engineMove) {
        if (engineMove) stdout.write(`Stockfish: ${san}\n`)
        persist()
      },
      flagged: () => persist(),
      failed(cause) {
        stderr.write(`Engine: ${errorSummary(cause)}\n`)
      },
    })
  }
  const input = createInterface({ input: stdin, output: stdout, terminal: stdin.isTTY })
  const game = computerGame ?? localGame
  const archiveHost = {
    source: () => game.archiveSnapshot(),
    hasPlay: () => game.state.moves.length > 0 || Boolean(game.result),
    games: () => [...games.values()],
    now: () => Date.now(),
    id: () => crypto.randomUUID(),
    save(entry: ArchivedGame) {
      const snapshot = structuredClone(entry)
      games.set(snapshot.id, snapshot)
      queue(() => core.saveArchivedGame(snapshot))
    },
    remove(id: string) {
      games.delete(id)
      queue(() => core.removeArchivedGame(id))
    },
  }
  const history = new GameArchive(
    archiveState(library.sessions[computer ? 'archive:computer' : 'archive:board'], archiveHost),
    archiveHost,
  )
  const show = (): void =>
    json({ fen: game.position.fen, moves: game.state.moves, result: game.result })
  const timer = setInterval(() => {
    if (game.expire()) {
      persist()
      show()
    }
  }, 100)
  const interrupted = (): void => {
    process.exitCode = 130
    computerGame?.dispose()
    input.close()
  }
  claimSignals()
  void core.finished.then(() => input.close())
  process.once('SIGINT', interrupted)
  process.once('SIGTERM', interrupted)
  try {
    show()
    if (computerGame) await computerGame.computerTurn()
    for await (const line of input) {
      const command = line.trim()
      if (command === 'quit') break
      if (command === 'status') show()
      else if (command === 'undo') game.takeback()
      else if (command === 'new') {
        history.reset()
        game.start()
      } else if (command === 'resign') {
        if (computerGame) computerGame.resign()
        else localGame.resign(localGame.position.turn)
      } else if (!computer && command === 'draw') localGame.agreeDraw()
      else if (!computer && command === 'pause') localGame.togglePause()
      else if (!game.move(command)) stderr.write('Illegal move or game unavailable.\n')
      if (computerGame) await computerGame.computerTurn()
      persist()
      show()
    }
  } finally {
    clearInterval(timer)
    input.close()
    computerGame?.dispose()
    persist()
    await save
    process.removeListener('SIGINT', interrupted)
    process.removeListener('SIGTERM', interrupted)
  }
}

export async function runCli(argv: string[]): Promise<void> {
  const { options, args, engine } = parseCli(argv)
  const command = args[0] ?? 'help'
  if (['help', '--help', '-h'].includes(command)) {
    stdout.write(HELP)
    return
  }
  if (command === 'methods') {
    json(CORE_METHODS)
    return
  }
  const core = await createNodeCore(options)
  let closing: Promise<void> | undefined
  const close = (): Promise<void> => (closing ??= core.close())
  const interruptSignal = new AbortController()
  const interrupted = (): void => {
    interruptSignal.abort()
    process.exitCode = 130
    void close().catch((cause) => {
      logWarn('cli', 'Shutdown failed:', errorSummary(cause))
    })
  }
  process.once('SIGINT', interrupted)
  process.once('SIGTERM', interrupted)
  try {
    if (engine) {
      await core.trustEnginePath(engine)
      await core.saveSettings({ ...(await core.settings()), enginePath: engine })
    }
    if (command === 'library') json(await core.library())
    else if (command === 'status')
      json({ data: await core.loadData(), engine: await core.engineStatus() })
    else if (command === 'login') json(await core.connectLichess())
    else if (command === 'call') {
      const method = args[1]
      if (!(CORE_METHODS as readonly string[]).includes(method ?? ''))
        throw new Error('Unknown CoreApi method. Run methods to list them.')
      if (args[2] === '--args-file' && (await stat(args[3] ?? '')).size > 2_000_000)
        throw new Error('Arguments are too large.')
      const text =
        args[2] === '--args-file' ? await readFile(args[3] ?? '', 'utf8') : (args[2] ?? '[]')
      if (text.length > 2_000_000) throw new Error('Arguments are too large.')
      const parameters: unknown = JSON.parse(text)
      if (!Array.isArray(parameters)) throw new Error('Arguments must be a JSON array.')
      if (
        [
          'bestMove',
          'startAnalysis',
          'reviewGet',
          'reviewRequest',
          'cloudEval',
          'positionLookup',
          'mastersGame',
        ].includes(method!)
      )
        await core.resumeOnline()
      const call = core[method as CoreMethod] as (...args: unknown[]) => Promise<unknown>
      json(await call(...parameters))
    } else if (command === 'analyze') {
      const fen = args[1]
      if (!fen) throw new Error('analyze requires a FEN.')
      await core.resumeOnline()
      core.on('engine:analysis', json)
      await core.startAnalysis({ fen, lines: 3 })
      if (!interruptSignal.signal.aborted)
        await Promise.race([
          core.finished,
          new Promise<void>((resolve) =>
            interruptSignal.signal.addEventListener('abort', () => resolve(), { once: true }),
          ),
        ])
    } else if (command === 'play' || command === 'local') {
      // Interactive input owns signals while the game persists its final snapshot.
      await interactive(core, command === 'play', args[1], () => {
        process.removeListener('SIGINT', interrupted)
        process.removeListener('SIGTERM', interrupted)
      })
    } else throw new Error(`Unknown command: ${command}. Run --help.`)
  } finally {
    process.removeListener('SIGINT', interrupted)
    process.removeListener('SIGTERM', interrupted)
    await close()
  }
}

if (process.argv[1] && /(?:^|[/\\])cli\.js$/.test(process.argv[1])) {
  try {
    await runCli(process.argv.slice(2))
  } catch (cause) {
    logWarn('cli', 'Command failed:', errorSummary(cause))
    process.exitCode = process.exitCode || 1
  }
}
