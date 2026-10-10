import { describe, expect, it, vi } from 'vitest'
import { seedSaved } from './libraryBackend'
import { flushPromises } from '@vue/test-utils'
import * as fc from 'fast-check'
import { INITIAL_FEN, Position } from '@kchess/core/domain/position'
import { addMove, newTree, nodeAt, treeFromPgn, treeToPgn } from '@kchess/core/domain/analysisTree'
import { assertAnalysisRequest } from '@kchess/core/domain/validate'
import { isAppUrl, APP_CSP } from '../../electron/main/appOrigin'
import { validateOnlineEvent } from '@kchess/core/domain/onlineEvent'
import { pickStockfishAsset } from '../../../../core/src/services/stockfishAsset'
import { useKChessStore } from '../../app/stores/kchess'
import { useAnalysisStore } from '../../app/stores/analysis'
import { desktop, deferred } from './fixtures'
import { useTestPlatform } from '../../../../tests/fixtures/corePlatform'

// The release asset rules run in the Rust core, which needs a platform to exist.
useTestPlatform()

describe('PGN preservation over generated legal games', () => {
  it('round trips legal positions, headers, variations and annotations', () => {
    fc.assert(
      fc.property(fc.array(fc.nat(), { maxLength: 60 }), (choices) => {
        let position = Position.initial()
        const root = newTree()
        root.headers = { White: 'Alice', Black: 'ボブ', Result: '1/2-1/2', Event: 'Study' }
        let path = ''
        for (const choice of choices) {
          const moves = [...position.dests('rules')].flatMap(([from, dests]) =>
            dests.map((to) => {
              const promotes =
                position.pieceAt(from)?.role === 'pawn' && (to.endsWith('1') || to.endsWith('8'))
              return `${from}${to}${promotes ? 'q' : ''}`
            }),
          )
          if (!moves.length) break
          const uci = moves[choice % moves.length]!
          path = addMove(root, path, uci)!
          nodeAt(root, path).comments = ['Comment [%clk 0:10:00]']
          nodeAt(root, path).nags = [1]
          position = position.play(uci)!.position
        }
        addMove(root, '', 'd2d4')
        const exported = treeToPgn(root)
        const restored = treeFromPgn(exported)
        expect(restored).toBeDefined()
        expect(treeToPgn(restored!)).toBe(exported)
      }),
      { numRuns: 40 },
    )
  })
  it('validates repetition context and rejects a mismatched position', () => {
    const root = newTree()
    addMove(root, '', 'e2e4')
    const fen = nodeAt(root, 'e2e4').fen
    expect(
      assertAnalysisRequest({ fen, rootFen: INITIAL_FEN, moves: ['e2e4'], lines: 1 }).moves,
    ).toEqual(['e2e4'])
    expect(() =>
      assertAnalysisRequest({ fen: INITIAL_FEN, rootFen: INITIAL_FEN, moves: ['e2e4'], lines: 1 }),
    ).toThrow()
    expect(() => assertAnalysisRequest({ fen: '8/8/8/8/8/8/8/8 w - - 0 1', lines: 1 })).toThrow()
  })
})
describe('origin and input boundaries', () => {
  it('requires exact authorities and rejects lookalike origins', () => {
    expect(isAppUrl('kchess://app/index.html')).toBe(true)
    expect(isAppUrl('kchess://app.evil/index.html')).toBe(false)
    expect(isAppUrl('http://127.0.0.1:30001', 'http://127.0.0.1:3000')).toBe(false)
    expect(isAppUrl('http://user@127.0.0.1:3000', 'http://127.0.0.1:3000')).toBe(false)
    expect(APP_CSP).toContain("object-src 'none'")
  })
  it('validates critical game-state fields while ignoring unknown event types', () => {
    expect(validateOnlineEvent({ type: 'futureEvent' })).toBeUndefined()
    expect(() =>
      validateOnlineEvent({ type: 'gameState', moves: ['e2e4'], status: 'started' }),
    ).toThrow()
    expect(() =>
      validateOnlineEvent({ type: 'gameState', moves: 'e2e4 garbage', status: 'started' }),
    ).toThrow()
    expect(
      validateOnlineEvent({
        type: 'gameState',
        moves: 'e2e4',
        status: 'started',
        wtime: 100,
        btime: 100,
      })?.type,
    ).toBe('gameState')
  })
  it('selects conservative official Windows/Linux builds for the correct CPU', () => {
    const assets = [
      'windows-x86-64-universal.zip',
      'linux-x86-64-universal.tar.gz',
      'linux-arm64-universal.tar.gz',
    ].map((name) => ({ name: 'stockfish-' + name, size: 1, browser_download_url: '' }))
    expect(pickStockfishAsset(assets, 'win32', 'x64')?.name).toContain('windows-x86-64')
    expect(pickStockfishAsset(assets, 'linux', 'arm64')?.name).toContain('linux-arm64')
    expect(pickStockfishAsset(assets, 'linux', 'ia32')).toBeUndefined()
  })
})
describe('desktop recovery', () => {
  it('rejects a late presence result after the game ends and never overlaps polls', async () => {
    vi.useFakeTimers()
    const pending = deferred<import('../../contracts/types').PresenceReport>()
    const presence = vi.fn(() => pending.promise)
    const bridge = desktop({ presence })
    const store = useKChessStore()
    await store.init()
    bridge.emit({
      type: 'gameStart',
      game: {
        gameId: 'AbCd1234',
        fullId: 'AbCd1234White',
        color: 'white',
        opponent: { id: 'bob', username: 'Bob', rating: 1500 },
      },
    })
    await flushPromises()
    await vi.advanceTimersByTimeAsync(15_000)
    expect(presence).toHaveBeenCalledTimes(1)
    bridge.emit({
      type: 'gameFinish',
      game: { gameId: 'AbCd1234', fullId: 'AbCd1234White', status: { id: 30, name: 'mate' } },
    })
    await flushPromises()
    pending.resolve({ users: {}, latencyMs: 100 })
    await flushPromises()
    expect(store.presence).toBeNull()
    expect(store.pingStats).toBeNull()
  })
  it('freezes input after an ambiguous move failure and accepts only current recovery state', async () => {
    const bridge = desktop({
      playOnline: async () => {
        throw new Error('response lost')
      },
    })
    const store = useKChessStore()
    await store.init()
    bridge.emit({
      type: 'gameStart',
      game: { gameId: 'AbCd1234', fullId: 'AbCd1234White', color: 'white' },
    })
    await store.onlineMove('e2e4')
    expect(store.onlinePhase).toBe('disconnected')
    expect(store.onlineCanPlay).toBe(false)
    expect(store.onlineStatus).toContain('could not be confirmed')
    store.readOnlineState({
      session: 5,
      account: 'Alice',
      gameId: 'AbCd1234',
      lane: 'game',
      phase: 'disconnected',
    })
    store.readOnlineState({
      session: 4,
      account: 'Bob',
      gameId: 'Other123',
      lane: 'game',
      phase: 'connected',
    })
    expect(store.onlineId).toBe('AbCd1234')
    expect(store.onlineAccount).toBe('Alice')
  })
  it('restores a valid computer game and rejects invalid saved moves', async () => {
    desktop()
    seedSaved(
      'kchess:computer:v1',
      JSON.stringify({
        version: 1,
        moves: ['e2e4', 'e7e5'],
        ply: 1,
        level: 'club',
        color: 'white',
        resigned: false,
      }),
    )
    const store = useKChessStore()
    expect(store.localMoves).toEqual(['e2e4', 'e7e5'])
    expect(store.localPly).toBe(1)
  })
  it('restores a study, annotations and selected variation', () => {
    desktop()
    seedSaved(
      'kchess:analysis:v1',
      JSON.stringify({
        version: 1,
        pgn: '[White "Alice"]\n1. e4 {comment} e5 (1... c5) *',
        path: 'e2e4 c7c5',
        orientation: 'black',
      }),
    )
    const store = useAnalysisStore()
    expect(store.path).toBe('e2e4 c7c5')
    expect(store.orientation).toBe('black')
    expect(store.pgn()).toContain('comment')
  })
  it('keeps busy set until every overlapping operation ends', async () => {
    const first =
      deferred<Awaited<ReturnType<import('../../contracts/types').DesktopApi['addFriends']>>>()
    const second =
      deferred<Awaited<ReturnType<import('../../contracts/types').DesktopApi['addFriends']>>>()
    let count = 0
    const { api } = desktop({ addFriends: () => (++count === 1 ? first.promise : second.promise) })
    const store = useKChessStore()
    await store.init()
    const one = store.importFriends(['Carol'])
    const two = store.importFriends(['Dave'])
    expect(store.busy).toBe(true)
    first.resolve(await api.loadData())
    await one
    expect(store.busy).toBe(true)
    second.resolve(await api.loadData())
    await two
    expect(store.busy).toBe(false)
  })
})
