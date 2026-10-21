import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import type { Color } from '@lichess-org/chessground/types'
import type { Judgment } from '@kchess/contracts/types'
import { playUci, positionFromFen } from '@kchess/rules/chess'
import { pathOf } from '@kchess/rules/analysisTree'
import { useAnalysisStore } from './analysis'
import { useMistakeStore } from './mistakes'

export type PracticeStatus = 'find' | 'wrong' | 'solved' | 'revealed' | 'done'

/**
 * Practising a reviewed game's mistakes on the analysis board, as Lichess's "learn from your
 * mistakes" does: the game's positions in practice, in move order, each with the game's moves and
 * chart around it. Answers schedule the positions as the Practice page does.
 */
export const useReviewPractice = defineStore('reviewPractice', () => {
  const analysis = useAnalysisStore()
  const mistakes = useMistakeStore()
  /** The review being practised. */
  const key = ref<string | null>(null)
  const index = ref(0)
  const status = ref<PracticeStatus>('find')
  /** The last wrong move tried here, as SAN. */
  const tried = ref('')
  const hinted = ref(false)
  /** A wrong move or a hint here: the position is not counted as found. */
  let helped = false
  /** Per position: found without help. */
  const found = ref<boolean[]>([])
  let orientation: Color = 'white'

  const active = computed(() => key.value !== null)
  const positions = computed(() => {
    const prefix = `${key.value}:`
    const ply = (id: string) => Number(id.slice(prefix.length))
    return key.value === null
      ? []
      : mistakes.items
          .filter((item) => item.id.startsWith(prefix))
          .map((item) => ({ ...item, ply: ply(item.id) }))
          .sort((a, b) => a.ply - b.ply)
  })
  const current = computed(() => positions.value[index.value])
  /** The move played instead: its numbered SAN, its label and who played it. */
  const played = computed(() => {
    const at = current.value
    const uci = at && analysis.mainline[at.ply]
    const pos = at && positionFromFen(at.fen)
    if (!at || !uci || !pos) return null
    const color = pos.turn
    const number = pos.fullmoves
    const san = playUci(pos, uci)
    if (!san) return null
    return {
      uci,
      label: `${number}${color === 'black' ? '…' : '.'} ${san}`,
      judgment: at.judgment as Judgment,
      color,
    }
  })
  /** The solution's first move: the one to find. */
  const answer = computed(() => current.value?.solution[0])
  const answerSan = computed(() => {
    const at = current.value
    const pos = at && positionFromFen(at.fen)
    return at && answer.value && pos ? playUci(pos, answer.value) || '' : ''
  })
  /** The board is at the position before the mistake (on the main line, which is the game). */
  const atPosition = computed(() => {
    const at = current.value
    return active.value && !!at && analysis.path === pathOf(analysis.mainline.slice(0, at.ply))
  })
  /** The game's move, drawn on the board while a better one is found. */
  const playedArrow = computed(() =>
    atPosition.value && hidesEngine.value ? played.value?.uci : undefined,
  )
  /** The piece to move, once a hint is asked for. */
  const hintSquare = computed(() =>
    hinted.value && atPosition.value && hidesEngine.value ? answer.value?.slice(0, 2) : undefined,
  )
  /** While a position is being solved, the engine (and anything else that knows) would tell. */
  const hidesEngine = computed(
    () => active.value && (status.value === 'find' || status.value === 'wrong'),
  )

  function show(): void {
    const at = current.value
    if (!at) return
    analysis.goToPly(at.ply)
    analysis.orientation = played.value?.color ?? analysis.orientation
  }
  function begin(): void {
    status.value = 'find'
    tried.value = ''
    hinted.value = false
    helped = false
    show()
  }
  function start(reviewKey: string): void {
    if (!active.value) orientation = analysis.orientation
    key.value = reviewKey
    index.value = 0
    found.value = []
    if (positions.value.length) begin()
    else stop()
  }
  /**
   * A move made on the board at the position being solved: true when practice took it (the answer
   * is played as a variation; a wrong move is not kept), false to play it as usual.
   */
  function attempt(uci: string): 'solved' | 'wrong' | null {
    const at = current.value
    if (!at || !atPosition.value || !hidesEngine.value) return null
    if (uci === answer.value) {
      found.value[index.value] = !helped
      void mistakes.answered(at.id, !helped)
      analysis.play(uci)
      status.value = 'solved'
      return 'solved'
    }
    const pos = positionFromFen(at.fen)
    tried.value = (pos && playUci(pos, uci)) || uci
    helped = true
    status.value = 'wrong'
    return 'wrong'
  }
  function hint(): void {
    hinted.value = true
    helped = true
    if (!atPosition.value) show()
  }
  function reveal(): void {
    const at = current.value
    if (!at || !answer.value) return
    found.value[index.value] = false
    void mistakes.answered(at.id, false)
    show()
    analysis.play(answer.value)
    status.value = 'revealed'
  }
  function next(): void {
    if (index.value + 1 >= positions.value.length) {
      status.value = 'done'
      return
    }
    index.value++
    begin()
  }
  function again(): void {
    if (key.value) start(key.value)
  }
  function stop(): void {
    if (!active.value) return
    key.value = null
    analysis.orientation = orientation
  }
  // Another game on the board, or a live game starting, ends practice. Moves added after the
  // end of the game keep its review, and so its practice.
  watch(
    () => analysis.mainlineKey,
    (now) => {
      if (active.value && now !== key.value && analysis.review?.key !== key.value) stop()
    },
  )
  watch(
    () => analysis.assistanceAllowed,
    (allowed) => {
      if (!allowed) stop()
    },
  )

  return {
    key,
    index,
    status,
    tried,
    hinted,
    found,
    active,
    positions,
    current,
    played,
    answer,
    answerSan,
    atPosition,
    hintSquare,
    playedArrow,
    hidesEngine,
    start,
    attempt,
    hint,
    reveal,
    next,
    again,
    stop,
    show,
  }
})
