import { expect, it, vi } from 'vitest'
import { treeFromPgn, treeToPgn } from '@kchess/rules/analysisTree'
import * as engine from '@kchess/rules/engine'

it('saves annotations, variations and custom headers without sending board snapshots', () => {
  const root = treeFromPgn(
    '[Event "Study"]\n[Result "*"]\n\n{Root} 1. e4 $1 {Note} (1. d4 {Variation}) e5 *',
  )!
  const headers = { ...root.headers, fen: 'custom header', uci: 'also a header', ply: '4' }
  const expected = engine.rules<string>('treeToPgn', root, headers)
  const invoke = vi.spyOn(engine, 'rules')
  expect(treeToPgn(root, headers)).toBe(expected)
  const [, document, sentHeaders] = invoke.mock.calls[0]!
  expect(sentHeaders).toEqual(headers)
  expect(document).toMatchObject({ fen: root.fen, comments: root.comments })
  const child = (document as { children: Record<string, unknown>[] }).children[0]!
  expect(child).toMatchObject({ san: 'e4', comments: ['Note'], nags: [1] })
  expect(child).not.toHaveProperty('fen')
  expect(child).not.toHaveProperty('uci')
  expect(child).not.toHaveProperty('ply')
  expect(JSON.stringify(document).length).toBeLessThan(JSON.stringify(root).length / 2)
})
