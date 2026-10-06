import { expect, it } from 'vitest'
import { nextTick } from 'vue'
import { useStudyStore } from '../../app/stores/studies'
import { useAnalysisStore } from '../../app/stores/analysis'
import { treeFromPgn } from '../../app/utils/analysisTree'
import { parsePgn } from 'chessops/pgn'
import { desktop } from './fixtures'

const chapters = [
  { name: 'King pawn', pgn: '[Event "King pawn"]\n\n1. e4 {Keep this} e5 (1... c5 $1) *' },
  { name: 'Queen pawn', pgn: '[Event "Queen pawn"]\n\n1. d4 d5 *' },
]

it('migrates existing single-game studies without changing their PGN', () => {
  localStorage.setItem(
    'kchess:studies:v1',
    JSON.stringify({
      version: 1,
      items: [{ id: 'legacy', name: 'Old study', pgn: chapters[0]!.pgn, updatedAt: 1 }],
    }),
  )
  const library = useStudyStore()
  expect(library.items[0]!.chapters).toEqual([
    { id: 'legacy', name: 'Old study', pgn: chapters[0]!.pgn },
  ])
  expect(library.items[0]!.pgn).toBe(chapters[0]!.pgn)
})

it('keeps all chapters together and edits the selected chapter without overwriting another', async () => {
  desktop()
  const library = useStudyStore()
  const id = library.saveChapters('Repertoire', chapters)
  const analysis = useAnalysisStore()
  const firstPgn = library.items[0]!.chapters[0]!.pgn
  const secondId = library.items[0]!.chapters[1]!.id
  expect(analysis.openStudy(id, secondId)).toBe(true)
  analysis.root.comments = ['Queen pawn notes']
  await nextTick()
  analysis.flushStudy()
  expect(library.items[0]!.chapters[0]!.pgn).toBe(firstPgn)
  expect(library.items[0]!.chapters[1]!.pgn).toContain('Queen pawn notes')
  expect(analysis.openStudy(id, library.items[0]!.chapters[0]!.id)).toBe(true)
  expect(analysis.pgn()).toContain('Keep this')
  expect(analysis.pgn()).toContain('$1')
  expect(analysis.pgn()).toContain('c5')
  library.rename(id, 'Renamed')
  const duplicate = library.duplicate(id)!
  expect(library.items.find((s) => s.id === duplicate)!.chapters).toHaveLength(2)
  const exported = parsePgn(library.documentPgn(library.items.find((s) => s.id === id)!))
  expect(exported.map((g) => g.headers.get('ChapterName'))).toEqual(['King pawn', 'Queen pawn'])
})

it('updates an unchanged offline copy without duplicates and keeps edited copies on download conflicts', () => {
  const library = useStudyStore()
  const remote = { id: 'Study001', name: 'Remote repertoire' }
  const initial = library.offline('Alice', remote, chapters)
  expect(library.items[0]!.cloud!.downloadedPgn).toBe(chapters.map((c) => c.pgn).join('\n\n'))
  const refreshed = library.offline('alice', remote, chapters)
  expect(refreshed.id).toBe(initial.id)
  expect(library.items).toHaveLength(1)
  const study = library.items[0]!
  library.save(study.name, '1. c4 *', study.id, study.chapters[1]!.id)
  const conflict = library.offline('Alice', remote, chapters)
  expect(conflict.conflict).toBe(true)
  expect(library.items).toHaveLength(2)
  expect(library.items.find((s) => s.id === initial.id)!.chapters[1]!.pgn).toContain('c4')
  expect(library.items.find((s) => s.id === conflict.id)!.chapters[1]!.pgn).toContain('d4')
})

it('rejects invalid downloads atomically, retaining the previous offline document', () => {
  const library = useStudyStore()
  const remote = { id: 'Study001', name: 'Remote repertoire' }
  library.offline('Alice', remote, chapters)
  const before = JSON.stringify(library.items)
  expect(treeFromPgn('1. e5 *')).toBeUndefined()
  expect(() =>
    library.offline('Alice', remote, [...chapters, { name: 'Broken', pgn: '1. e5 *' }]),
  ).toThrow()
  expect(JSON.stringify(library.items)).toBe(before)
})

it('duplicates chapter annotations and removes chapters without changing surviving identities', () => {
  const library = useStudyStore()
  const id = library.saveChapters('Study', chapters)
  const [first, second] = library.items[0]!.chapters
  const copy = library.duplicateChapter(id, first!.id)
  expect(library.items[0]!.chapters.map((c) => c.id)).toEqual([first!.id, second!.id, copy])
  expect(library.items[0]!.chapters[2]!.pgn).toBe(first!.pgn)
  const next = library.removeChapter(id, first!.id)
  expect(next).toBe(second!.id)
  expect(library.items[0]!.pgn).toBe(second!.pgn)
  library.removeChapter(id, copy)
  expect(() => library.removeChapter(id, second!.id)).toThrow('at least one chapter')
})
