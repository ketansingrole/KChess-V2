import { rules } from './engine.ts'
import lessons from '../../kchess-domain/data/lessons.json' with { type: 'json' }

export type LessonGoal = 'mate' | 'win' | 'draw' | 'reach' | 'confine'

/** What a step asks of you (`crates/kchess-domain/src/training/lessons.rs`). */
export type LessonTask =
  | { kind: 'read' }
  /** Your moves at even indices, the scripted replies between; the last may be any mate. */
  | { kind: 'line'; moves: string[] }
  /** Force checkmate within `within` of your moves; the lesson plays the defence. */
  | { kind: 'mate'; within: number }
  /** Play the goal out against the engine within `within` of your moves. */
  | { kind: 'play'; goal: LessonGoal; within: number; squares?: string[] }

export interface LessonShape {
  orig: string
  dest?: string
  brush: 'green' | 'red' | 'blue' | 'yellow'
}

export interface LessonStep {
  fen: string
  text: string
  shapes?: LessonShape[]
  task: LessonTask
}

export interface LessonChapter {
  /** Unique across all courses; it is the saved run's variant. */
  id: string
  title: string
  steps: LessonStep[]
}

export type CourseGroup = 'Checkmates' | 'Pawn endgames' | 'Rook endgames'

export interface LessonCourse {
  id: string
  group: CourseGroup
  title: string
  subtitle: string
  chapters: LessonChapter[]
}

/** The courses (`data/lessons.json`), shared with the Rust rules, which check every step. */
export const LESSON_COURSES = lessons.courses as readonly LessonCourse[]
export const COURSE_GROUPS: readonly CourseGroup[] = [
  'Checkmates',
  'Pawn endgames',
  'Rook endgames',
]

export type LessonReason =
  | 'read'
  | 'lineDone'
  | 'checkmate'
  | 'mated'
  | 'stalemate'
  | 'insufficient'
  | 'repetition'
  | 'fiftyMoves'
  | 'promoted'
  | 'bareKing'
  | 'reached'
  | 'confined'
  | 'held'
  | 'outOfMoves'
  | 'materialLost'
  | 'lost'

export interface LessonStatus {
  over: boolean
  /** Only once `over`. */
  success?: boolean
  reason?: LessonReason
  /** The side that solves the step. */
  player: 'white' | 'black'
  turn: 'white' | 'black'
  fen: string
  check: boolean
  /** Your moves so far, and the most the task allows. */
  played: number
  limit?: number
  /** The opponent's next move when the lesson knows it. */
  reply?: string
  /** The opponent is to move and the engine must answer. */
  engine: boolean
}

/** Where `task`, from `fen`, stands after `moves` (UCI). */
export function lessonStatus(
  fen: string,
  task: LessonTask,
  moves: readonly string[],
): LessonStatus {
  return rules('lessonStatus', fen, task, [...moves])
}

/** Whether the step accepts `uci` as your next move, and the move as played. */
export function lessonMove(
  fen: string,
  task: LessonTask,
  moves: readonly string[],
  uci: string,
): { accepted: boolean; uci?: string } {
  return rules('lessonMove', fen, task, [...moves], uci)
}

/** The move a line or mate step wants next; null for play steps (ask the engine). */
export function lessonHint(fen: string, task: LessonTask, moves: readonly string[]): string | null {
  return rules('lessonHint', fen, task, [...moves])
}

/** Chapters finished, out of the course's chapters, given the finished chapter ids. */
export function courseProgress(
  course: LessonCourse,
  done: (chapterId: string) => boolean,
): { done: number; total: number } {
  return { done: course.chapters.filter((c) => done(c.id)).length, total: course.chapters.length }
}

/** What is wrong with the course data: illegal or unsolvable steps, bad ids. Empty when sound. */
export function lessonProblems(): string[] {
  return rules('lessonProblems')
}
