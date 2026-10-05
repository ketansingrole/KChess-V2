import * as v from 'valibot'
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
