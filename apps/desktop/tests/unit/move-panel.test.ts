import { expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import MovePanel from '../../app/components/MovePanel.vue'
import { componentStubs } from './fixtures'

it('numbers the moves from the starting position, also when Black moves first', () => {
  // Every list was numbered from White's first move, whatever the position.
  const panel = mount(MovePanel, {
    props: {
      moves: ['Kd7', 'Ke2', 'Kd6'],
      ply: 0,
      fen: '8/4k3/8/8/8/8/8/4K3 b - - 0 12',
    },
    global: { stubs: componentStubs },
  })
  const numbers = panel.findAll('.move-num').map((cell) => cell.text())
  expect(numbers).toEqual(['12', '13'])
  const labels = panel.findAll('button.move-cell').map((cell) => cell.attributes('aria-label'))
  expect(labels).toEqual(['Move 12, black, Kd7', 'Move 13, white, Ke2', 'Move 13, black, Kd6'])
  expect(panel.text()).toContain('2 moves')
  const standard = mount(MovePanel, {
    props: { moves: ['e4', 'e5', 'Nf3'], ply: 0 },
    global: { stubs: componentStubs },
  })
  expect(standard.findAll('.move-num').map((cell) => cell.text())).toEqual(['1', '2'])
  expect(standard.text()).toContain('2 moves')
})
