import { describe, expect, it } from 'vitest'
import {
  COURSE_GROUPS,
  LESSON_COURSES,
  courseProgress,
  lessonHint,
  lessonMove,
  lessonProblems,
  lessonStatus,
  type LessonStep,
  type LessonTask,
} from '@kchess/rules/lessons'
import { assertRunInput } from '@kchess/rules/validate'

const steps = LESSON_COURSES.flatMap((course) =>
  course.chapters.flatMap((chapter) =>
    chapter.steps.map((step, index) => ({ at: `${chapter.id} #${index + 1}`, step })),
  ),
)
const of = (kind: LessonTask['kind']) => steps.filter(({ step }) => step.task.kind === kind)

/** Plays the step as the lesson expects (its hints, and its replies), and returns the end. */
function solve(step: LessonStep) {
  const moves: string[] = []
  for (let ply = 0; ply < 40; ply++) {
    const status = lessonStatus(step.fen, step.task, moves)
    if (status.over) return status
    if (status.turn === status.player) {
      const hint = lessonHint(step.fen, step.task, moves)
      expect(hint, 'a hint while it is your move').toBeTruthy()
      const verdict = lessonMove(step.fen, step.task, moves, hint!)
      expect(verdict.accepted, `the hint ${hint} is accepted`).toBe(true)
      moves.push(verdict.uci!)
    } else {
      expect(status.reply, 'the lesson knows the reply').toBeTruthy()
      moves.push(status.reply!)
    }
  }
  throw new Error('The step did not finish.')
}

describe('lesson courses', () => {
  it('has every Lichess checkmate, pawn and rook course, in its group', () => {
    expect(LESSON_COURSES.map((course) => course.title)).toEqual([
      'Piece Checkmates I',
      'Checkmate Patterns I',
      'Checkmate Patterns II',
      'Checkmate Patterns III',
      'Checkmate Patterns IV',
      'Piece Checkmates II',
      'Knight & Bishop Mate',
      'Key Squares',
      'Opposition',
      '7th-Rank Pawn',
      '7th-Rank Rook Pawn',
      'Basic Rook Endgames',
      'Intermediate Rook Endings',
      'Practical Rook Endings',
    ])
    for (const course of LESSON_COURSES) expect(COURSE_GROUPS).toContain(course.group)
  })

  it('passes the Rust checks: legal moves, forced mates, valid squares', () => {
    expect(lessonProblems()).toEqual([])
  })

  it('names chapters with ids that are unique and valid run variants', () => {
    const ids = LESSON_COURSES.flatMap((course) => [course.id, ...course.chapters.map((c) => c.id)])
    expect(new Set(ids).size).toBe(ids.length)
    for (const course of LESSON_COURSES)
      for (const chapter of course.chapters)
        expect(
          assertRunInput({ kind: 'lesson', variant: chapter.id, score: 1, detail: {} }).variant,
        ).toBe(chapter.id)
  })

  it('solves every line and mate step with its own hints and replies', () => {
    for (const { at, step } of [...of('line'), ...of('mate')]) {
      const end = solve(step)
      expect(end.success, at).toBe(true)
      if (step.task.kind === 'mate') expect(end.reason, at).toBe('checkmate')
    }
  })

  it('starts every play step on your move, not yet decided', () => {
    for (const { at, step } of of('play')) {
      const status = lessonStatus(step.fen, step.task, [])
      expect(status.over, at).toBe(false)
      expect(status.turn, at).toBe(status.player)
      expect(status.engine, at).toBe(false)
      expect(lessonHint(step.fen, step.task, []), at).toBeNull()
    }
  })

  it('ends read steps at once', () => {
    for (const { at, step } of of('read'))
      expect(lessonStatus(step.fen, step.task, []), at).toMatchObject({
        over: true,
        success: true,
        reason: 'read',
      })
  })
})

describe('lesson rules', () => {
  const backRank: LessonTask = { kind: 'mate', within: 2 }
  const fen = '3r2k1/5ppp/8/8/8/8/4RPPP/4R1K1 w - - 0 1'

  it('rejects a move that lets the mate slip and answers with the longest defence', () => {
    expect(lessonMove(fen, backRank, [], 'h2h3')).toEqual({ accepted: false })
    expect(lessonMove(fen, backRank, [], 'e2e8')).toEqual({ accepted: true, uci: 'e2e8' })
    expect(lessonStatus(fen, backRank, ['e2e8'])).toMatchObject({
      over: false,
      reply: 'd8e8',
      engine: false,
      played: 1,
      limit: 2,
    })
    expect(lessonStatus(fen, backRank, ['e2e8', 'd8e8', 'e1e8'])).toMatchObject({
      over: true,
      success: true,
      reason: 'checkmate',
    })
  })

  it('asks the engine to answer play steps', () => {
    const task: LessonTask = { kind: 'play', goal: 'mate', within: 12 }
    const start = '8/8/8/4k3/8/8/8/R2QK3 w - - 0 1'
    expect(lessonMove(start, task, [], 'd1d4').accepted).toBe(true)
    expect(lessonStatus(start, task, ['d1d4'])).toMatchObject({ engine: true, over: false })
  })

  it('fails a goal that runs out of moves and a draw that is lost', () => {
    const reach: LessonTask = { kind: 'play', goal: 'reach', within: 1, squares: ['c5'] }
    expect(lessonStatus('8/8/8/8/8/3K4/3P4/7k w - - 0 1', reach, ['d3e3', 'h1g1'])).toMatchObject({
      over: true,
      success: false,
      reason: 'outOfMoves',
    })
    const draw: LessonTask = { kind: 'play', goal: 'draw', within: 5 }
    expect(lessonStatus('8/8/8/8/8/k7/1q6/7K w - - 0 1', draw, ['h1g1'])).toMatchObject({
      over: true,
      success: false,
      reason: 'lost',
    })
  })

  it('counts finished chapters for a course badge', () => {
    const course = LESSON_COURSES[0]!
    const first = course.chapters[0]!.id
    expect(courseProgress(course, (id) => id === first)).toEqual({
      done: 1,
      total: course.chapters.length,
    })
  })

  it('reports an invalid lesson position', () => {
    expect(() => lessonStatus('nonsense', { kind: 'read' }, [])).toThrow('Invalid lesson position.')
  })
})
