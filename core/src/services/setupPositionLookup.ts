import { scopedState } from './platform'
import { logDebug } from './logger'
import { lichessFetch } from './requestPolicy'
import { nativeCallSync } from './nativeCore'
import { PositionLookupService, decodeLookupCache } from './positionLookup'
import { getToken } from './store'

const serviceState = scopedState(() => ({
  positionLookups: new PositionLookupService(
    {
      read(key) {
        // The stored entry's text, or null when it is missing or too large to read (Rust core).
        const data = nativeCallSync<string | null>('store.setupPositionLookup.read', key)
        if (data === null) return undefined
        try {
          return decodeLookupCache(JSON.parse(data))
        } catch (cause) {
          logDebug('position', 'Position lookup cache entry is invalid:', key, cause)
          return undefined
        }
      },
      write(key, value) {
        nativeCallSync('store.setupPositionLookup.write', key, value)
      },
    },
    async (input, init) => {
      const request = new Request(input, init)
      // The official explorer now requires OAuth. Credentials never leave main or go to tablebases.
      if (new URL(request.url).hostname === 'explorer.lichess.org') {
        const account = nativeCallSync<string | null>('store.setupPositionLookup.explorerAccount')
        const token = account && (await getToken(account))
        if (!token)
          throw new Error('Connect a Lichess account in Settings to use the opening explorer.')
        request.headers.set('Authorization', `Bearer ${token}`)
      }
      return lichessFetch(request)
    },
  ),
}))
export const positionLookups = scopedState(() => serviceState.positionLookups)
