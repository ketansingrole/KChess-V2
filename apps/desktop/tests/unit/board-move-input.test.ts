import { expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import type { SquareName } from '@kchess/rules/position'
import { INITIAL_FEN } from '@kchess/rules/position'
import ChessBoard from '../../app/components/ChessBoard.vue'

const dests = new Map<SquareName, SquareName[]>([['e2', ['e3', 'e4']]])

// The box went away on the other side's turn and at the end of a game, moving everything under
// the board.
it('keeps the typed-move box of a board played on, read-only while no move can be made', async () => {
  const preview = mount(ChessBoard, { props: { fen: INITIAL_FEN, interactive: false } })
  expect(preview.find('form.board-move-form').exists()).toBe(false)
  preview.unmount()

  const board = mount(ChessBoard, {
    props: { fen: INITIAL_FEN, keepMoveInput: true, movable: false, interactive: false },
    attachTo: document.body,
  })
  await flushPromises()
  const input = () => board.get<HTMLInputElement>('input.board-move-input')
  expect(board.get('form.board-move-form').classes()).toContain('idle')
  expect(input().element.readOnly).toBe(true)
  expect(input().attributes('aria-disabled')).toBe('true')
  input().element.focus()
  await board.get('form.board-move-form').trigger('submit')
  expect(board.emitted('move')).toBeUndefined()

  // Its turn again: the box still has the focus, and a typed move plays.
  await board.setProps({ movable: true, interactive: true, dests })
  expect(document.activeElement).toBe(input().element)
  expect(input().element.readOnly).toBe(false)
  expect(board.get('form.board-move-form').classes()).not.toContain('idle')
  await input().setValue('e4')
  await board.get('form.board-move-form').trigger('submit')
  expect(board.emitted('move')).toEqual([['e2e4']])
  board.unmount()
})
