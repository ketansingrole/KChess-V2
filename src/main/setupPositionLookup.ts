import { getDb } from './db'
import { logDebug } from './logger'
import { lichessFetch } from './requestPolicy'
import { PositionLookupService, decodeLookupCache } from './positionLookup'
import { getToken } from './store'

export const positionLookups = new PositionLookupService(
  {
    read(key) {
      const row = getDb().prepare('SELECT data FROM position_lookups WHERE key = ?').get(key) as
        { data: string } | undefined
      if (!row || row.data.length > 512_000) return undefined
      try {
        return decodeLookupCache(JSON.parse(row.data))
      } catch (cause) {
        logDebug('position', 'Position lookup cache entry is invalid:', key, cause)
        return undefined
      }
    },
    write(key, value) {
      const db = getDb()
      db.prepare(
        'INSERT OR REPLACE INTO position_lookups (key, data, fetchedAt) VALUES (?, ?, ?)',
      ).run(key, JSON.stringify(value), value.fetchedAt)
      db.exec(
        'DELETE FROM position_lookups WHERE key NOT IN (SELECT key FROM position_lookups ORDER BY fetchedAt DESC LIMIT 256)',
      )
    },
  },
  async (input, init) => {
    const request = new Request(input, init)
    // The official explorer now requires OAuth. Credentials never leave main or go to tablebases.
    if (new URL(request.url).hostname === 'explorer.lichess.org') {
      const account = getDb()
        .prepare('SELECT username FROM accounts WHERE connected = 1 ORDER BY username LIMIT 1')
        .get() as { username: string } | undefined
      const token = account && (await getToken(account.username))
      if (!token)
        throw new Error('Connect a Lichess account in Settings to use the opening explorer.')
      request.headers.set('Authorization', `Bearer ${token}`)
    }
    return lichessFetch(request)
  },
)
