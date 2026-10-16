import { onUnmounted, ref, shallowRef, watch, type Ref } from 'vue'
import { addMove, newTree, nodeAt, type TreeNode } from '@kchess/rules/analysisTree'
import { positionFromFen } from '@kchess/rules/chess'
import { openingReplies } from '@kchess/rules/openings'
import { RequestScope } from '@kchess/rules/requestScope'
import { errorText } from '../utils/format'

/** An opening drilled without a saved study: its name and its moves from the starting position. */
export interface OpeningChoice {
  name: string
  moves: string[]
}

/** How far past the named line the drill goes, in half-moves… */
const EXTRA_PLIES = 6
/** …and how many positions it asks the explorer about to get there. */
const LOOKUP_BUDGET = 16

/**
 * The tree an opening is drilled from: its named line, then the explorer's popular moves for a
 * few more moves (the most played one for the player, the popular replies for the opponent).
 * Without the explorer (offline, or no Lichess account) the named line alone is drilled, and
 * `note` says why. Lookups are the explorer's saved ones when they are fresh.
 */
export function useOpeningDrill(choice: Ref<OpeningChoice | null>, color: Ref<'white' | 'black'>) {
  const tree = shallowRef<TreeNode | null>(null)
  const preparing = ref(false)
  const note = ref('')
  const scope = new RequestScope()

  async function build(): Promise<void> {
    const request = scope.next()
    tree.value = null
    note.value = ''
    const opening = choice.value
    if (!opening) return
    const root = newTree()
    let path = ''
    for (const uci of opening.moves) {
      const next = addMove(root, path, uci)
      if (next === undefined) break
      path = next
    }
    preparing.value = true
    const queue = [{ path, depth: 0 }]
    let lookups = 0
    try {
      for (let entry = queue.shift(); entry; entry = queue.shift()) {
        if (lookups++ >= LOOKUP_BUDGET) break
        const node = nodeAt(root, entry.path)
        let moves
        try {
          moves = (await window.kchess.positionLookup('opening', node.fen, {})).moves
        } catch (cause) {
          console.warn('[opening-drill] Looking up popular replies failed:', cause)
          if (!request.current()) return
          // The named line is still worth drilling; only a drill with nothing past it says why.
          if (entry.path === path) note.value = errorText(cause)
          break
        }
        if (!request.current()) return
        const yours = positionFromFen(node.fen)?.turn === color.value
        for (const uci of openingReplies(moves, yours)) {
          const child = addMove(root, entry.path, uci)
          if (child !== undefined && entry.depth + 1 < EXTRA_PLIES)
            queue.push({ path: child, depth: entry.depth + 1 })
        }
      }
      tree.value = root
    } finally {
      if (request.current()) preparing.value = false
    }
  }

  watch([choice, color], () => void build(), { immediate: true, deep: true })
  onUnmounted(() => scope.invalidate())
  return { tree, preparing, note }
}
