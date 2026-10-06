import * as v from 'valibot'
import { parsePgn, makePgn, startingPosition } from 'chessops/pgn'
import { parseSan } from 'chessops/san'
import type { Position } from 'chessops/chess'
import type { ChildNode, PgnNodeData } from 'chessops/pgn'
import type { LichessStudy, LichessStudyChapter, NeedsReconnect } from '../shared/types'
import { asAccount, authorize, client, unwrap, urlencoded } from './lichess'
import { readLines } from './ndjson'

const MAX_STUDY_PGN = 4_000_000
const metadata = v.looseObject({
  id: v.pipe(v.string(), v.regex(/^[a-zA-Z0-9]{8}$/)),
  name: v.pipe(v.string(), v.maxLength(200)),
  createdAt: v.optional(v.number()),
  updatedAt: v.optional(v.number()),
})

/** The studies an account owns or belongs to (private ones too, with `study:read`). */
export function lichessStudies(account: string): Promise<LichessStudy[] | NeedsReconnect> {
  return asAccount(
    account,
    async (token) => {
      const stream = await unwrap(
        client.GET('/api/study/by/{username}', {
          params: { path: { username: account } },
          headers: { ...authorize(token), Accept: 'application/x-ndjson' },
          parseAs: 'stream',
        }),
      )
      const studies: LichessStudy[] = []
      await readLines(stream, (line) => {
        const parsed = v.safeParse(metadata, JSON.parse(line))
        if (parsed.success && studies.length < 300)
          studies.push({
            id: parsed.output.id,
            name: parsed.output.name,
            updatedAt: parsed.output.updatedAt ?? parsed.output.createdAt ?? 0,
          })
      })
      return studies.sort((a, b) => b.updatedAt - a.updatedAt)
    },
    'study',
  )
}

/** Split a multi-game PGN into games (Lichess separates them with blank lines before a tag). */
export function splitPgn(text: string): string[] {
  return text
    .replace(/\r\n/g, '\n')
    .split(/\n\s*\n(?=\[)/)
    .map((part) => part.trim())
    .filter((part) => part.startsWith('['))
}

/** Every chapter of a study, with variations, comments and NAGs, ready for the analysis board. */
export function lichessStudyChapters(
  account: string,
  id: string,
): Promise<LichessStudyChapter[] | NeedsReconnect> {
  return asAccount(
    account,
    async (token) => {
      const text = await unwrap(
        client.GET('/api/study/{studyId}.pgn', {
          params: {
            path: { studyId: id },
            query: { comments: true, variations: true, clocks: false, orientation: true },
          },
          headers: { ...authorize(token), Accept: 'application/x-chess-pgn' },
          parseAs: 'text',
        }) as Promise<{ data?: string | null; response: Response }>,
      )
      if (text.length > MAX_STUDY_PGN) throw new Error('That study is too large to import.')
      return splitPgn(text)
        .slice(0, 64)
        .map((pgn, index) => {
          const tag = (name: string): string | undefined =>
            new RegExp(`^\\[${name} "([^"]*)"\\]`, 'm').exec(pgn)?.[1]
          return {
            name: (tag('ChapterName') ?? tag('Event') ?? `Chapter ${index + 1}`).slice(0, 120),
            pgn,
          }
        })
    },
    'study',
  )
}

/**
 * Add a game to a Lichess study as new chapters, or to a new study when `studyId` is empty.
 * Returns the study id.
 */
export function exportToLichessStudy(
  account: string,
  studyId: string,
  name: string,
  pgn: string,
): Promise<{ id: string } | NeedsReconnect> {
  return asAccount(
    account,
    async (token) => {
      const headers = { ...authorize(token), 'Content-Type': 'application/x-www-form-urlencoded' }
      let id = studyId
      if (!id) {
        const created = await unwrap(
          client.POST('/api/study', {
            body: {
              name,
              visibility: 'private',
              computer: 'everyone',
              explorer: 'everyone',
              cloneable: 'owner',
              shareable: 'owner',
              chat: 'member',
            },
            bodySerializer: urlencoded,
            headers,
          }),
        )
        if (!created.id) throw new Error('Lichess did not create the study.')
        id = created.id
      }
      await unwrap(
        client.POST('/api/study/{studyId}/import-pgn', {
          params: { path: { studyId: id } },
          body: { pgn, name },
          bodySerializer: urlencoded,
          headers,
        }),
      )
      return { id }
    },
    'study',
  )
}

/** Explicit upload of edits to a linked study; concurrent cloud edits are never overwritten. */
export async function syncLichessStudy(
  request: import('../shared/types').StudySyncRequest,
): Promise<LichessStudyChapter[] | NeedsReconnect> {
  const before = validChapters(request.baseline)
  const after = validChapters(request.pgn)
  if (!before.length || before.length > after.length || before.length > 64)
    throw new Error('Chapter structure changed. Upload a new cloud copy instead.')
  const current = await lichessStudyChapters(request.account, request.studyId)
  if ('needsReconnect' in current) return current
  const remote = validChapters(current.map((c) => c.pgn).join('\n\n'))
  if (
    remote.length !== before.length ||
    remote.some((game, index) => makePgn(game) !== makePgn(before[index]!))
  )
    throw new Error(
      'The cloud study changed. Download its latest copy before uploading; your offline edits are kept.',
    )
  const ids = before.map((game) => {
    const site = game.headers.get('Site') ?? ''
    const match = /\/study\/([a-zA-Z0-9]{8})\/([a-zA-Z0-9]{8})(?:[?#].*)?$/.exec(site)
    if (!match || match[1] !== request.studyId)
      throw new Error('Missing cloud chapter identity. Download the study again first.')
    return match[2]!
  })
  const result = await asAccount(
    request.account,
    async (token) => {
      for (let index = 0; index < before.length; index++) {
        const game = after[index]!
        if (makePgn(game) === makePgn(before[index]!)) continue
        const params = { path: { studyId: request.studyId, chapterId: ids[index]! } }
        const headers = { ...authorize(token), 'Content-Type': 'application/x-www-form-urlencoded' }
        await unwrap(
          client.POST('/api/study/{studyId}/{chapterId}/moves', {
            params,
            headers,
            bodySerializer: urlencoded,
            body: { pgn: makePgn(game) },
          }),
        )
        const tags = new Map(game.headers)
        // Chapter identity belongs to Lichess, rather than an edited local PGN.
        tags.set('Site', before[index]!.headers.get('Site')!)
        // Removing a local PGN tag must also remove it from the cloud chapter.
        for (const key of before[index]!.headers.keys()) if (!tags.has(key)) tags.set(key, '')
        const tagsPgn = [...tags]
          .map(([key, value]) => `[${key} "${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"]`)
          .join('\n')
        await unwrap(
          client.POST('/api/study/{studyId}/{chapterId}/tags', {
            params,
            headers,
            bodySerializer: urlencoded,
            body: { pgn: tagsPgn },
          }),
        )
      }
      for (const game of after.slice(before.length)) {
        await unwrap(
          client.POST('/api/study/{studyId}/import-pgn', {
            params: { path: { studyId: request.studyId } },
            headers: { ...authorize(token), 'Content-Type': 'application/x-www-form-urlencoded' },
            bodySerializer: urlencoded,
            body: {
              pgn: makePgn(game),
              name: game.headers.get('ChapterName') ?? game.headers.get('Event') ?? 'New chapter',
            },
          }),
        )
      }
      return { id: request.studyId }
    },
    'study',
  )
  if ('needsReconnect' in result) return result
  return lichessStudyChapters(request.account, request.studyId)
}

function validChapters(pgn: string) {
  const games = parsePgn(pgn)
  if (!games.length || games.length > 64) throw new Error('Use up to 64 valid chapters.')
  for (const game of games) {
    const start = startingPosition(game.headers)
    if (start.isErr) throw new Error('Invalid chapter starting position.')
    const stack: { node: ChildNode<PgnNodeData>; pos: Position }[] = game.moves.children.map(
      (node) => ({ node, pos: start.value.clone() }),
    )
    while (stack.length) {
      const { node, pos } = stack.pop()!
      const move = parseSan(pos, node.data.san)
      if (!move) throw new Error('The chapter contains an illegal move.')
      pos.play(move)
      for (const child of node.children) stack.push({ node: child, pos: pos.clone() })
    }
  }
  return games
}
