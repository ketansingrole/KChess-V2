import { expect, it, vi } from 'vitest'
import { defineComponent, nextTick } from 'vue'
import { flushPromises, shallowMount } from '@vue/test-utils'
import { desktop, deferred } from './fixtures'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useStudyStore } from '../../app/stores/studies'
import { useKChessStore } from '../../app/stores/kchess'
import * as tree from '../../app/utils/analysisTree'
import AnalysisPage from '../../app/pages/analysis.vue'
import { makeUci } from 'chessops/util'
import { positionFromFen } from '../../app/utils/chess'

it('hides cloud scores, variations, arrows and playable suggestions during live play and remount', async () => {
  desktop({ reviewGet: async () => null, onAnalysis: () => () => {}, stopAnalysis: async () => {} })
  const app = useKChessStore()
  await app.init()
  const store = useAnalysisStore()
  store.engineOn = false
  const Board = defineComponent({ props: ['shapes'], template: '<div />' })
  const stubs = Object.fromEntries(
    [
      'PageHeader',
      'StudyBar',
      'EvalBar',
      'UButton',
      'UIcon',
      'UContextMenu',
      'UDropdownMenu',
      'UFormField',
      'UModal',
      'UPopover',
      'USelect',
      'USwitch',
      'UTooltip',
      'VoiceInput',
      'AnalysisLine',
      'AnalysisReview',
      'ExportGame',
      'MoveNotes',
      'PositionExplorer',
    ].map((name) => [name, true]),
  )
  const mountPage = () =>
    shallowMount(AnalysisPage, { global: { stubs: { ...stubs, ChessBoard: Board } } })
  let page = mountPage()
  store.cloud = {
    fen: store.node.fen,
    depth: 20,
    knodes: 1,
    lines: [{ rank: 1, depth: 20, cp: 30, pv: ['e2e4'] }],
  }
  await nextTick()
  expect(page.find('.pv-eval').text()).toBe('+0.3')
  expect(page.find('.pv-move').text()).toContain('e4')
  expect(page.findComponent(Board).props('shapes')).toHaveLength(1)
  for (const phase of ['playing', 'seeking', 'disconnected'] as const) {
    app.onlinePhase = phase
    await nextTick()
    expect(page.find('eval-bar-stub').exists()).toBe(false)
    expect(page.find('.pv-eval').exists()).toBe(false)
    expect(page.find('.pv-move').exists()).toBe(false)
    expect(page.findComponent(Board).props('shapes')).toEqual([])
  }
  page.unmount()
  page = mountPage()
  await nextTick()
  expect(page.find('eval-bar-stub').exists()).toBe(false)
  expect(page.find('.pv-eval').exists()).toBe(false)
  expect(page.findComponent(Board).props('shapes')).toEqual([])
  page.unmount()
})

it('clears cached cloud assistance while detached and rejects a late cloud response', async () => {
  vi.useFakeTimers()
  const pending = deferred<null>()
  desktop({
    reviewGet: async () => null,
    onAnalysis: () => () => {},
    stopAnalysis: async () => {},
    cloudEval: () => pending.promise,
  })
  const app = useKChessStore()
  await app.init()
  app.settings.cloudEval = true
  const store = useAnalysisStore()
  store.engineOn = false
  store.attach()
  await vi.advanceTimersByTimeAsync(300)
  store.cloud = {
    fen: store.node.fen,
    depth: 20,
    knodes: 1,
    lines: [{ rank: 1, depth: 20, cp: 30, pv: ['e2e4'] }],
  }
  store.detach()
  app.onlinePhase = 'playing'
  expect(store.cloud).toBeNull()
  expect(store.assistanceAllowed).toBe(false)
  pending.resolve(null)
  await flushPromises()
  store.attach()
  await nextTick()
  expect(store.cloud).toBeNull()
  expect(store.cloudBusy).toBe(false)
  store.detach()
})
it('preserves a conflicting restored session as an unsaved copy', async () => {
  vi.useFakeTimers()
  desktop({ reviewGet: async () => null })
  localStorage.setItem(
    'kchess:studies:v1',
    JSON.stringify({
      version: 1,
      items: [{ id: 's', name: 'Study', pgn: '1. d4 *', updatedAt: 2 }],
    }),
  )
  localStorage.setItem(
    'kchess:analysis:v1',
    JSON.stringify({ version: 1, study: 's', pgn: '1. e4 *', path: '', orientation: 'white' }),
  )
  const store = useAnalysisStore(),
    studies = useStudyStore()
  expect(store.studyId).toBe('')
  expect(store.studySaveError).toContain('unsaved copy')
  store.forward()
  store.node.comments = ['Recovered edit']
  await vi.advanceTimersByTimeAsync(200)
  expect(studies.items[0]?.pgn).toBe('1. d4 *')
  expect(JSON.parse(localStorage.getItem('kchess:analysis:v1')!).pgn).toContain('Recovered edit')
})
it.each(['kchess:studies:v1', 'kchess:analysis:v1'])(
  'preserves the previous document and surfaces a failed write to %s',
  async (failedKey) => {
    vi.useFakeTimers()
    desktop({ reviewGet: async () => null })
    const store = useAnalysisStore()
    store.loadPgn('1. e4 *')
    store.saveAsStudy('Study')
    await vi.advanceTimersByTimeAsync(200)
    const before = localStorage.getItem(failedKey)
    const write = localStorage.setItem.bind(localStorage)
    const fail = vi.spyOn(localStorage, 'setItem').mockImplementation((key, value) => {
      if (key === failedKey) throw new Error('Storage unavailable')
      write(key, value)
    })
    store.forward()
    store.node.comments = ['Recover this edit']
    await vi.advanceTimersByTimeAsync(200)
    expect(localStorage.getItem(failedKey)).toBe(before)
    expect(failedKey.includes('studies') ? store.studySaveError : store.saveError).toContain(
      'Storage unavailable',
    )
    const otherKey = failedKey.includes('studies') ? 'kchess:analysis:v1' : 'kchess:studies:v1'
    expect(localStorage.getItem(otherKey)).toContain('Recover this edit')
    fail.mockRestore()
  },
)
it('flushes pending edits before opening another study and before unload', async () => {
  desktop({ reviewGet: async () => null })
  const store = useAnalysisStore(),
    studies = useStudyStore()
  store.loadPgn('1. e4 *')
  const first = store.saveAsStudy('One')
  const second = studies.save('Two', '1. d4 *')
  store.forward()
  store.node.comments = ['Keep before switch']
  store.openStudy(second)
  expect(studies.items.find((s) => s.id === first)?.pgn).toContain('Keep before switch')
  store.forward()
  store.node.comments = ['Keep before quit']
  window.dispatchEvent(new Event('beforeunload'))
  expect(
    JSON.parse(localStorage.getItem('kchess:studies:v1')!).items.find(
      (s: { id: string }) => s.id === second,
    ).pgn,
  ).toContain('Keep before quit')
})
it('reuses the document PGN while navigating and debounces serialization of edits', async () => {
  vi.useFakeTimers()
  desktop({ reviewGet: async () => null })
  const serialize = vi.spyOn(tree, 'treeToPgn')
  const store = useAnalysisStore()
  const branching = tree.newTree()
  let nodes = 0
  const branch = (path: string, depth: number): void => {
    if (!depth) return
    const pos = positionFromFen(tree.nodeAt(branching, path).fen)!
    const moves = [...pos.allDests()]
      .flatMap(([from, squares]) => [...squares].map((to) => makeUci({ from, to })))
      .slice(0, 6)
    for (const uci of moves) {
      nodes++
      branch(tree.addMove(branching, path, uci)!, depth - 1)
    }
  }
  branch('', 5)
  expect(nodes).toBe(9330)
  store.loadPgn(tree.treeToPgn(branching))
  await vi.advanceTimersByTimeAsync(200)
  serialize.mockClear()
  for (let i = 0; i < 20; i++) {
    store.forward()
    await nextTick()
    store.toStart()
    await nextTick()
  }
  await vi.advanceTimersByTimeAsync(200)
  expect(serialize).not.toHaveBeenCalled()
  store.forward()
  store.node.comments = ['one']
  await nextTick()
  store.node.comments = ['two']
  await nextTick()
  expect(serialize).not.toHaveBeenCalled()
  await vi.advanceTimersByTimeAsync(200)
  expect(serialize).toHaveBeenCalledTimes(1)
  expect(store.pgn()).toContain('two')
})

it('hides assistance during startup recovery and after failure until a confirmed retry', async () => {
  const query = deferred<null>()
  const startAnalysis = vi.fn(async () => 1)
  const { api } = desktop({
    resumeOnline: () => query.promise,
    startAnalysis,
    onAnalysis: () => () => {},
    stopAnalysis: async () => {},
    reviewGet: async () => null,
  })
  const app = useKChessStore()
  await app.init()
  const analysis = useAnalysisStore()
  analysis.attach()
  await flushPromises()
  expect(analysis.assistanceAllowed).toBe(false)
  expect(startAnalysis).not.toHaveBeenCalled()
  query.reject(new Error('offline'))
  await flushPromises()
  expect(analysis.assistanceAllowed).toBe(false)
  expect(app.onlineStatus).toBe('offline')
  api.resumeOnline = async () => null
  await app.reconnectOnline()
  await flushPromises()
  expect(analysis.assistanceAllowed).toBe(true)
  expect(startAnalysis).toHaveBeenCalled()
  analysis.detach()
})

it('closing a study saves pending chapter edits and clears the board and move list', async () => {
  desktop()
  const library = useStudyStore()
  const id = library.saveChapters('Two chapters', [
    { name: 'First', pgn: '1. e4 *' },
    { name: 'Second', pgn: '1. d4 d5 (1... Nf6 $1) *' },
  ])
  const analysis = useAnalysisStore()
  analysis.openStudy(id, library.items[0]!.chapters[1]!.id)
  analysis.root.comments = ['Keep the pending edit']
  await nextTick()
  expect(analysis.closeStudy()).toBe(true)
  expect(analysis.studyId).toBe('')
  expect(analysis.studyChapterId).toBe('')
  expect(analysis.root.children).toHaveLength(0)
  expect(analysis.path).toBe('')
  await nextTick()
  expect(library.items[0]!.chapters[1]!.pgn).toContain('Keep the pending edit')
  expect(library.items[0]!.chapters[1]!.pgn).toContain('Nf6 $1')
  expect(library.items[0]!.chapters[0]!.pgn).toBe('1. e4 *')
  expect(JSON.parse(localStorage.getItem('kchess:studies:v1')!).items[0].chapters[1].pgn).toContain(
    'Keep the pending edit',
  )
})

it('keeps the study and board open when pending edits cannot be persisted', async () => {
  desktop()
  const library = useStudyStore()
  const id = library.save('Do not lose', '1. e4 *')
  const analysis = useAnalysisStore()
  analysis.openStudy(id)
  analysis.root.comments = ['Unsaved edit']
  await nextTick()
  vi.spyOn(localStorage, 'setItem').mockImplementation(() => {
    throw new Error('Storage full')
  })
  expect(analysis.closeStudy()).toBe(false)
  expect(analysis.studyId).toBe(id)
  expect(analysis.root.children).toHaveLength(1)
  expect(analysis.studySaveError).toContain('Storage full')
})
