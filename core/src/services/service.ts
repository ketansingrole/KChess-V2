import { coreData, coreSettings } from '../contracts/settings'
import { realpathSync } from 'node:fs'
import { resolve } from 'node:path'
import { validateCoreArguments } from '../contracts/apiContracts'
import { CORE_METHODS, type CoreMethod, type CoreEvents } from '../contracts/core'
import type {
  ChallengeInfo,
  CoreApi,
  CoreSettings,
  OnlineEvent,
  Settings,
} from '../contracts/types'
import {
  assertAction,
  assertActivityMax,
  assertAnalysisRequest,
  assertBestMoveOptions,
  assertBroadcastQuery,
  assertChatRoom,
  assertChatText,
  assertDays,
  assertDeclineReason,
  assertFriendList,
  assertGameId,
  assertGameIds,
  assertGamePageQuery,
  assertInsightsQuery,
  assertLadderQuery,
  assertLevel,
  assertLichessId,
  assertLocalQuery,
  assertMessageText,
  assertMoves,
  assertNewArena,
  assertOnlineOptions,
  assertOptionalAccount,
  assertPerfType,
  assertPuzzleRequest,
  assertPuzzleSolve,
  assertReviewKey,
  assertReviewRequest,
  assertRunInput,
  assertRunKind,
  assertSettings,
  assertCoreSettings,
  assertStudySyncRequest,
  assertTournamentId,
  assertTournamentPassword,
  assertTournamentSystem,
  assertUci,
  assertUsername,
  assertUsernames,
  assertVoiceAttempt,
  assertVoiceId,
  assertVoiceLimit,
  assertVoiceUpdate,
  assertWatchTarget,
} from '../domain/validate'
import {
  assertArchivedGame,
  assertLegacyDocuments,
  assertLibraryId,
  assertRepertoireKey,
  assertSession,
  assertSessionKind,
  assertSide,
  assertStudyCommand,
} from '../domain/library'
import { logWarn } from './logger'
import { analysisRunning, startAnalysis, stopAnalysis } from './analysis'
import { ChallengeInbox } from './challenges'
import { cloudEval, clearCloudEval } from './cloudEval'
import { closeDb } from './db'
import {
  bestMove,
  computerPlaying,
  engineIdentity,
  engineStatus,
  isTrustedEnginePath,
  stopEngine,
  trustEnginePath,
  resetEngine,
} from './engine'
import { configureEngineResources } from './engineScheduler'
import { insights } from './insights'
import {
  addMistakes,
  answerMistake,
  clearRepertoireMisses,
  forgetTournament,
  forgetTournamentsOf,
  importLibrary,
  joinedTournaments,
  library,
  recordRepertoireMiss,
  rememberTournament,
  removeArchivedGame,
  saveArchivedGame,
  saveSession,
  studyCommand,
} from './library'
import {
  OnlineSession,
  cachedProfile,
  cancelAccountSyncs,
  connectLichess,
  crosstable,
  exportGame,
  fetchLichessReviews,
  followedUsers,
  forgetProfile,
  invalidateLogin,
  playerPerf,
  primeProfiles,
  profile,
  puzzleActivity,
  puzzleDaily,
  puzzleDashboard,
  puzzleNext,
  puzzleSolve,
  ratingHistory,
  recentGames,
  resetLichess,
  sendMessage,
  stormDashboard,
  syncGames,
} from './lichess'
import { deleteManagedEngine, installManagedEngine } from './managedEngine'
import { oauthLook } from '../domain/oauthLook'
import {
  runWithPlatform,
  bindPlatform,
  abortPlatform,
  closePlatform,
  type CorePlatform,
} from './platform'
import {
  cancelPuzzleDb,
  closePuzzleWorker,
  deletePuzzleDb,
  installPuzzleDb,
  localLadder,
  localPuzzles,
  puzzleDbStatus,
} from './puzzleDb'
import {
  cancelReview,
  discardAccountReviews,
  getReview,
  requestReview,
  restartReviewEngine,
  reviewStatus,
  reviewsChanged,
  setupReviews,
  stopReviews,
} from './review'
import { reviewSummaries } from './reviewStore'
import { clearRuns, runSummary, saveRun } from './runs'
import { positionLookups } from './setupPositionLookup'
import { Spectator, broadcastTour, broadcasts, tvChannels } from './spectate'
import { TV_CHANNEL_KEYS } from '../domain/tvChannels'
import {
  addAccount,
  addFriends,
  clearAccountData,
  gameLibraryOverview,
  gamePage,
  gamePgn,
  gameRatingHistory,
  getSettings,
  loadData,
  logoutAccounts,
  removeAccount,
  saveSettings,
  resetStore,
} from './store'
import {
  exportToLichessStudy,
  lichessStudies,
  lichessStudyChapters,
  syncLichessStudy,
} from './studies'
import {
  createTournament,
  joinTournament,
  leaveTournament,
  tournament,
  tournaments,
} from './tournaments'
import { closeEngines, withEngineMaintenance } from './uci'
import { closeUsage, flushUsage, forgetUsage, resetUsage, usageReport } from './usage'
import {
  clearVoiceHistory,
  saveVoiceAttempt,
  updateVoiceAttempt,
  voiceHistory,
  voiceHistoryDocument,
} from './voiceLog'

type Listener<K extends keyof CoreEvents> = (payload: CoreEvents[K]) => void

/** The headless KChess services behind any frontend. */
export interface KChessCore extends CoreApi {
  on<K extends keyof CoreEvents>(event: K, listener: Listener<K>): () => void
  /** The saved settings, for frontend features outside the core. */
  settings(): Promise<CoreSettings>
  /** Desktop adapter compatibility; headless clients never receive these preferences. */
  desktopSettings(): Promise<Settings>
  saveDesktopSettings(settings: Settings): Promise<Settings>
  /** Allow an engine executable the user picked themselves to be saved in settings. */
  trustEnginePath(path: string): void
  /** The voice log as a JSON document, for the frontend to save. */
  voiceHistoryDocument(): string
  /** Stop live work nothing can answer without a frontend; services stay usable. */
  suspend(): void
  /** Stop everything and close the databases. */
  close(): Promise<void>
}

/**
 * Configure an independent core for a host and start its scoped services.
 * Distinct profiles can coexist in one runtime. Engine, analysis,
 * review and lookup help stay unavailable until `resumeOnline()` has confirmed that no live
 * Lichess game is in progress.
 */
const profiles = new Set<string>()

export function createKChessCore(platform: CorePlatform): KChessCore {
  const profileDirectory = realpathSync(resolve(platform.dataDir))
  if (profiles.has(profileDirectory))
    throw new Error('A KChess core is already running for this profile.')
  profiles.add(profileDirectory)
  try {
    return runWithPlatform(platform, () => createCore(platform, profileDirectory))
  } catch (cause) {
    profiles.delete(profileDirectory)
    throw cause
  }
}

function createCore(platform: CorePlatform, profileDirectory: string): KChessCore {
  const inScope = bindPlatform(<T>(work: () => T): T => work())
  configureEngineResources(() => platform.onBattery())

  let closed = false
  let closing: Promise<void> | undefined
  const pending = new Set<Promise<unknown>>()
  const listeners = new Map<keyof CoreEvents, Set<(payload: never) => void>>()
  function emit<K extends keyof CoreEvents>(event: K, payload: CoreEvents[K]): void {
    if (closed) return
    for (const listener of listeners.get(event) ?? []) {
      try {
        ;(listener as Listener<K>)(payload)
      } catch (cause) {
        logWarn('core', 'Event listener failed:', event, cause)
      }
    }
  }

  const challengeInbox = new ChallengeInbox((list) => emit('challenges:update', list))
  let ongoingTimer: ReturnType<typeof setTimeout> | undefined
  /** Several games start and end together when a stream (re)connects; report them once. */
  function ongoingChanged(): void {
    clearTimeout(ongoingTimer)
    ongoingTimer = setTimeout(() => emit('online:ongoing-changed', null), 750)
  }
  const online = new OnlineSession(
    (event: OnlineEvent) => emit('online:event', event),
    (message: string) => emit('online:error', message),
    (state) => {
      if (state.phase === 'checking' || (state.gameId && state.phase !== 'idle')) {
        stopEngine()
        stopAnalysis()
        restartReviewEngine()
        // Your own game needs the connection more than someone else's.
        spectator.stop()
      }
      emit('online:state', state)
    },
    {
      challenge: (account, event) => {
        const fresh = challengeInbox.ingest(account, event)
        if (fresh) emit('challenge:received', fresh)
      },
      ongoingChanged,
      lobbyState: (state) => emit('online:lobby', state),
    },
  )
  const spectator = new Spectator(
    (frame) => emit('watch:frame', frame),
    (update) => emit('watch:broadcast', update),
    (state) => emit('watch:state', state),
  )
  function knownChallenge(id: unknown): ChallengeInfo {
    const challenge = challengeInbox.get(assertGameId(id))
    if (!challenge) throw new Error('That challenge is no longer open.')
    return challenge
  }
  function assistanceAllowed(message: string): void {
    if (online.assistanceBlocked) throw new Error(message)
  }

  /** Ends everything a signed-in account has running: login, syncs, live play, reviews, usage. */
  async function endSessions(username?: string): Promise<string[]> {
    const accounts = (await loadData()).accounts
      .filter(
        (a) => a.connected && (!username || a.username.toLowerCase() === username.toLowerCase()),
      )
      .map((a) => a.username)
    invalidateLogin(accounts)
    online.logout(username === undefined ? undefined : accounts)
    discardAccountReviews(accounts)
    flushUsage()
    forgetUsage(accounts)
    for (const name of accounts) challengeInbox.forget(name)
    // Tournaments joined as a signed-out account would otherwise reopen its event streams.
    forgetTournamentsOf(accounts)
    return accounts
  }
  async function logout(username?: string) {
    await endSessions(username)
    const data = await logoutAccounts(username)
    reviewsChanged()
    return data
  }
  const replaceEngineFile = (commit: () => Promise<void>): Promise<void> =>
    withEngineMaintenance(commit, () => {
      stopEngine(true)
      stopAnalysis(true)
      restartReviewEngine()
    })

  bindPlatform(() =>
    setupReviews({
      settings: async () => coreSettings(await getSettings()),
      accounts: async () => {
        const { accounts } = await loadData()
        // Your own accounts: the connected ones, or the first one added when none is.
        const own = accounts.filter((account) => account.connected)
        return (own.length ? own : accounts.slice(0, 1)).map((account) => account.username)
      },
      onBattery: () => platform.onBattery(),
      // The computer opponent counts as busy for a minute after its move: the game goes on.
      busy: () =>
        online.assistanceBlocked
          ? 'online'
          : analysisRunning() || computerPlaying(60_000)
            ? 'engine'
            : undefined,
      fetchLichess: fetchLichessReviews,
      update: (update) => emit('review:update', update),
      status: (status) => emit('review:status', status),
    }),
  )()

  async function persistSettings(settings: Settings): Promise<Settings> {
    const previous = await getSettings()
    // A frontend may not point the core at an arbitrary executable: only a path the user
    // picked, downloaded by KChess, or already saved is accepted.
    if (
      settings.enginePath &&
      !isTrustedEnginePath(settings.enginePath) &&
      settings.enginePath !== previous.enginePath
    )
      throw new Error('Choose the Stockfish executable with the file picker.')
    const saved = await saveSettings(settings)
    if (previous.enginePath !== saved.enginePath) {
      stopEngine(true)
      stopAnalysis(true)
      restartReviewEngine()
    }
    emit('settings:saved', coreSettings(saved))
    reviewsChanged()
    return saved
  }

  const implementation: KChessCore = {
    on(event, listener) {
      let set = listeners.get(event)
      if (!set) listeners.set(event, (set = new Set()))
      set.add(listener)
      return () => void set.delete(listener)
    },
    settings: async () => coreSettings(await getSettings()),
    trustEnginePath,
    voiceHistoryDocument,
    suspend() {
      cancelPuzzleDb()
      spectator.stop()
      // Nothing can answer a challenge without a frontend; it asks to stay connected again.
      online.close()
      stopEngine(true)
      stopAnalysis(true)
    },
    close() {
      if (closing) return closing
      closed = true
      clearTimeout(ongoingTimer)
      spectator.stop()
      online.close()
      stopEngine(true)
      stopAnalysis(true)
      const reviews = stopReviews()
      flushUsage()
      abortPlatform()
      closePlatform()
      listeners.clear()
      const puzzles = closePuzzleWorker()
      closing = (async () => {
        await Promise.allSettled([...pending, reviews, puzzles])
        await closeEngines().catch((cause) => logWarn('core', 'Engine shutdown failed:', cause))
        closeDb()
        resetStore()
        resetLichess()
        clearCloudEval()
        resetEngine()
        closeUsage()
        profiles.delete(profileDirectory)
      })()
      return closing
    },

    loadData: async () => loadData(),
    async saveSettings(raw) {
      const settings = assertCoreSettings(raw)
      return coreSettings(await persistSettings({ ...(await getSettings()), ...settings }))
    },
    desktopSettings: getSettings,
    saveDesktopSettings: async (raw) => persistSettings(assertSettings(raw)),
    addAccount: async (username) => addAccount(assertUsername(username)),
    logout: async (username) => logout(assertUsername(username)),
    logoutAll: async () => logout(),
    async removeAccount(username) {
      const name = assertUsername(username)
      // A followed player has no login, but its sync and reviews must still stop before rows go.
      if (!(await endSessions(name)).length) {
        cancelAccountSyncs([name])
        discardAccountReviews([name])
        flushUsage()
        forgetUsage([name])
        forgetTournamentsOf([name])
      }
      challengeInbox.forget(name)
      const data = await removeAccount(name)
      reviewsChanged()
      return data
    },
    async syncGames(username) {
      const data = await syncGames(username === undefined ? undefined : assertUsername(username))
      reviewsChanged()
      return data
    },
    gamePage: async (query) => gamePage(assertGamePageQuery(query)),
    gameLibraryOverview: async () => gameLibraryOverview(),
    insights: async (query) => insights(assertInsightsQuery(query)),
    syncLichessStudy: async (request) => syncLichessStudy(assertStudySyncRequest(request)),
    lichessStudies: async (account) => lichessStudies(assertUsername(account)),
    lichessStudyChapters: async (account, id) =>
      lichessStudyChapters(assertUsername(account), assertLichessId(id)),
    async exportToLichessStudy(account, studyId, name: unknown, pgn: unknown) {
      const id = studyId === '' ? '' : assertLichessId(studyId)
      if (typeof name !== 'string' || !name.trim() || name.length > 100)
        throw new Error('Name the chapter (up to 100 characters).')
      if (typeof pgn !== 'string' || !pgn.trim() || pgn.length > 500_000)
        throw new Error('Nothing to export, or the game is too large.')
      return exportToLichessStudy(assertUsername(account), id, name.trim(), pgn)
    },
    gameRatingHistory: async (account) => gameRatingHistory(assertUsername(account)),
    gamePgn: async (account, id) => gamePgn(assertUsername(account), assertGameId(id)),
    cachedProfile: async (username) => cachedProfile(assertUsername(username)),
    profile: async (username) => profile(assertUsername(username)),
    ratingHistory: async (username) => ratingHistory(assertUsername(username)),
    async connectLichess(look) {
      const connected = await connectLichess(oauthLook(look), () => platform.focus?.())
      platform.focus?.()
      return connected
    },
    async engineStatus() {
      const status = await engineStatus((await getSettings()).enginePath)
      return { ...status, identity: status.ready ? await engineIdentity(status) : undefined }
    },
    async installEngine() {
      const { path, version, updated } = await installManagedEngine({
        replace: replaceEngineFile,
      })
      return { path, version, updated }
    },
    deleteEngine: async () => deleteManagedEngine(undefined, replaceEngineFile),
    stopEngine: async () => stopEngine(),
    async bestMove(moves, level, options) {
      assistanceAllowed('Engine assistance is unavailable during a live Lichess game.')
      return bestMove(
        assertMoves(moves),
        assertLevel(level),
        (await getSettings()).enginePath,
        assertBestMoveOptions(options),
      )
    },
    async startAnalysis(request) {
      assistanceAllowed('Analysis is unavailable during a live Lichess game.')
      return startAnalysis(request, (await getSettings()).enginePath, (update) =>
        emit('engine:analysis', update),
      )
    },
    async positionLookup(kind, fen, options) {
      assistanceAllowed('Position lookups are unavailable during a live Lichess game.')
      return positionLookups.lookup(kind, fen, options)
    },
    exportGame: async (id) => exportGame(assertGameId(id)),
    async mastersGame(id) {
      assistanceAllowed('Position lookups are unavailable during a live Lichess game.')
      return positionLookups.mastersGame(id)
    },
    async cloudEval(fen, lines) {
      assistanceAllowed('Analysis is unavailable during a live Lichess game.')
      // Each request sends the position to Lichess, so it needs the setting turned on.
      if (!(await getSettings()).cloudEval)
        throw new Error('Turn on cloud evaluation in Settings → Analysis first.')
      const request = assertAnalysisRequest({ fen, lines })
      return cloudEval(request.fen, request.lines)
    },
    stopAnalysis: async () => stopAnalysis(),
    async reviewGet(fen, moves) {
      assistanceAllowed('Review is unavailable until your Lichess game status is verified.')
      const request = assertReviewRequest({ fen, moves })
      return getReview(request.fen, request.moves)
    },
    async reviewRequest(request) {
      assistanceAllowed('Review is unavailable during a live Lichess game.')
      return requestReview(assertReviewRequest(request))
    },
    reviewCancel: async (key) => cancelReview(assertReviewKey(key)),
    reviewStatus: async () => reviewStatus(),
    reviewSummaries: async (ids) => reviewSummaries(assertGameIds(ids)),
    startOnline: async (options) => online.start(assertOnlineOptions(options)),
    resumeOnline: async () => online.resume(),
    cancelOnline: async () => online.cancel(),
    playOnline: async (id, move) => online.move(assertGameId(id), assertUci(move)),
    onlineAction: async (id, action) => online.action(assertGameId(id), assertAction(action)),
    presence: async (usernames) => online.presence(assertUsernames(usernames)),
    onlineChat: async (id) => online.chat(assertGameId(id)),
    sendChat: async (id, room, text) =>
      online.sendChat(assertGameId(id), assertChatRoom(room), assertChatText(text)),
    async stayConnected(account) {
      const name = assertOptionalAccount(account)
      const connected = (await loadData()).accounts.some(
        (entry) => entry.connected && entry.username.toLowerCase() === name.toLowerCase(),
      )
      online.stayConnected(connected ? name : '')
    },
    challenges: async () => challengeInbox.list(),
    async acceptChallenge(id) {
      const challenge = knownChallenge(id)
      if (challenge.direction !== 'in' || !challenge.playable)
        throw new Error(challenge.problem ?? 'Only challenges sent to you can be accepted.')
      await online.acceptChallenge(challenge)
      challengeInbox.remove(challenge.id)
    },
    async declineChallenge(id, reason) {
      const challenge = knownChallenge(id)
      if (challenge.direction !== 'in') throw new Error('Withdraw your own challenge instead.')
      await online.declineChallenge(challenge, assertDeclineReason(reason))
      challengeInbox.remove(challenge.id)
    },
    async cancelChallenge(id) {
      const challenge = knownChallenge(id)
      if (challenge.direction !== 'out') throw new Error('Decline a challenge sent to you instead.')
      await online.withdrawChallenge(challenge)
      challengeInbox.remove(challenge.id)
    },
    ongoingGames: async () => online.ongoing(),
    openGame: async (account, id) => online.open(assertUsername(account), assertGameId(id)),
    tournaments: async (account) => tournaments(assertOptionalAccount(account)),
    tvChannels: async () => tvChannels(),
    playerPerf: async (username, perf) =>
      playerPerf(assertUsername(username), assertPerfType(perf)),
    crosstable: async (a, b) => crosstable(assertUsername(a), assertUsername(b)),
    recentGames: async (username, rated) => recentGames(assertUsername(username), rated === true),
    sendMessage: async (account, username, text) =>
      sendMessage(assertUsername(account), assertUsername(username), assertMessageText(text)),
    async watch(target) {
      if (online.playing) throw new Error('Finish your game before watching another.')
      return spectator.watch(assertWatchTarget(target, TV_CHANNEL_KEYS))
    },
    async watchBroadcast(roundId) {
      if (online.playing) throw new Error('Finish your game before watching another.')
      return spectator.watchRound(assertLichessId(roundId))
    },
    stopWatching: async () => spectator.stop(),
    broadcasts: async (query) => broadcasts(assertBroadcastQuery(query)),
    broadcastTour: async (id) => broadcastTour(assertLichessId(id)),
    async tournament(rawSystem, rawId, account, page: unknown) {
      const system = assertTournamentSystem(rawSystem)
      const id = assertTournamentId(rawId)
      const detail = await tournament(
        system,
        id,
        assertOptionalAccount(account),
        typeof page === 'number' && Number.isInteger(page) && page >= 1 && page <= 200 ? page : 1,
      )
      // An arena says whether you are in it; forget it when it is over or you left.
      if (system === 'arena' && (detail.status === 'finished' || !detail.me || detail.me.withdraw))
        forgetTournament(system, id)
      return detail
    },
    async joinTournament(rawSystem, rawId, account, password) {
      const name = assertUsername(account)
      const system = assertTournamentSystem(rawSystem)
      const id = assertTournamentId(rawId)
      const result = await joinTournament(system, id, name, assertTournamentPassword(password))
      if (result !== true) return result
      // Pairings arrive on the event stream: keep it open while in the tournament.
      online.stayConnected(name)
      // Arenas end at `finishesAt`; Swiss events have no fixed end, so allow a day.
      let entry = { system, id, account: name, name: id, until: Date.now() + 86_400_000 }
      try {
        const detail = await tournament(system, id, name, 1)
        entry = { ...entry, name: detail.name, until: detail.finishesAt ?? entry.until }
      } catch (cause) {
        logWarn('tournaments', 'Joined tournament details are unavailable:', `id=${id}`, cause)
      }
      rememberTournament(entry)
      return result
    },
    async leaveTournament(rawSystem, rawId, account) {
      const system = assertTournamentSystem(rawSystem)
      const id = assertTournamentId(rawId)
      const result = await leaveTournament(system, id, assertUsername(account))
      if (result === true) forgetTournament(system, id)
      return result
    },
    createTournament: async (account, arena) =>
      createTournament(assertUsername(account), assertNewArena(arena)),
    async clearAccountData(username) {
      const name = assertUsername(username)
      // Stop a running sync and review first, or they would write back into the cleared library.
      cancelAccountSyncs([name])
      discardAccountReviews([name])
      const data = await clearAccountData(name)
      forgetProfile(name)
      reviewsChanged()
      return data
    },
    following: async () => followedUsers(),
    async addFriends(usernames) {
      const names = assertFriendList(usernames)
      const data = await addFriends(names)
      await primeProfiles(names)
      return data
    },
    puzzleNext: async (request) => puzzleNext(assertPuzzleRequest(request)),
    puzzleSolve: async (request) => puzzleSolve(assertPuzzleSolve(request)),
    puzzleDaily: async () => puzzleDaily(),
    puzzleDashboard: async (account, days) =>
      puzzleDashboard(assertUsername(account), assertDays(days)),
    puzzleActivity: async (account, max) =>
      puzzleActivity(assertUsername(account), assertActivityMax(max)),
    stormDashboard: async (username, days) =>
      stormDashboard(assertUsername(username), assertDays(days)),
    puzzleDbStatus: async () => puzzleDbStatus(),
    puzzleDbInstall: async () => installPuzzleDb((progress) => emit('puzzledb:progress', progress)),
    puzzleDbCancel: async () => cancelPuzzleDb(),
    puzzleDbDelete: async () => deletePuzzleDb(),
    localPuzzles: async (query) => localPuzzles(assertLocalQuery(query)),
    localLadder: async (query) => localLadder(assertLadderQuery(query)),
    saveRun: async (run) => saveRun(assertRunInput(run)),
    runSummary: async (kind) => runSummary(assertRunKind(kind)),
    clearRuns: async (kind) => clearRuns(kind === undefined ? undefined : assertRunKind(kind)),
    saveVoiceAttempt: async (attempt) => saveVoiceAttempt(assertVoiceAttempt(attempt)),
    updateVoiceAttempt: async (id, update) =>
      updateVoiceAttempt(assertVoiceId(id), assertVoiceUpdate(update)),
    voiceHistory: async (limit) => voiceHistory(assertVoiceLimit(limit)),
    clearVoiceHistory: async () => clearVoiceHistory(),
    library: async () => library(),
    importLibrary: async (documents) => importLibrary(assertLegacyDocuments(documents)),
    studyCommand: async (command) => studyCommand(assertStudyCommand(command)),
    saveArchivedGame: async (game) => saveArchivedGame(assertArchivedGame(game)),
    removeArchivedGame: async (id) => removeArchivedGame(assertLibraryId(id)),
    addMistakes: async (key, color) =>
      addMistakes(assertReviewKey(key), color === undefined ? undefined : assertSide(color)),
    answerMistake: async (id, solved: unknown) => {
      if (typeof solved !== 'boolean') throw new Error('Invalid answer.')
      return answerMistake(assertLibraryId(id), solved)
    },
    async saveSession(rawKind, value) {
      const kind = assertSessionKind(rawKind)
      saveSession(kind, assertSession(kind, value))
    },
    joinedTournaments: async () => joinedTournaments(),
    recordRepertoireMiss: async (key, fen) =>
      recordRepertoireMiss(assertRepertoireKey(key), assertAnalysisRequest({ fen, lines: 1 }).fen),
    clearRepertoireMisses: async (key) => clearRepertoireMisses(assertRepertoireKey(key)),
    usage: async () => usageReport(),
    resetUsage: async () => resetUsage(),
  }
  // Capture the host once: stale callbacks and references cannot target a later profile.
  const bound = new Map<PropertyKey, unknown>()
  return new Proxy(implementation, {
    get(target, key, receiver) {
      const value = Reflect.get(target, key, receiver)
      if (typeof value !== 'function') return value
      if (!bound.has(key))
        bound.set(key, (...args: unknown[]) =>
          inScope(() => {
            if (closed && key !== 'close') {
              const error = new Error('The KChess core is closed.')
              if (
                key === 'on' ||
                key === 'trustEnginePath' ||
                key === 'suspend' ||
                key === 'voiceHistoryDocument'
              )
                throw error
              return Promise.reject(error)
            }
            if ((CORE_METHODS as readonly PropertyKey[]).includes(key)) {
              try {
                validateCoreArguments(key as CoreMethod, args)
              } catch (cause) {
                logWarn('core', 'Invalid method arguments:', String(key), cause)
                return Promise.reject(cause)
              }
            }
            const rawResult: unknown = value(...args)
            const project = (result: unknown): unknown => {
              if (
                result &&
                typeof result === 'object' &&
                'settings' in result &&
                'accounts' in result
              )
                return coreData(result as import('../contracts/types').AppData)
              if (
                key === 'connectLichess' &&
                result &&
                typeof result === 'object' &&
                'data' in result
              )
                return {
                  ...result,
                  data: coreData((result as { data: import('../contracts/types').AppData }).data),
                }
              return result
            }
            const result =
              key === 'close' || key === 'desktopSettings' || key === 'saveDesktopSettings'
                ? rawResult
                : rawResult instanceof Promise
                  ? rawResult.then(project)
                  : project(rawResult)
            if (result instanceof Promise && key !== 'close') {
              pending.add(result)
              // Both exits remove ownership without creating an unhandled rejection.
              void result.then(
                () => pending.delete(result),
                () => pending.delete(result),
              )
            }
            return result
          }),
        )
      return bound.get(key)
    },
  })
}
