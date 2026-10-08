import { expect, it } from 'vitest'
import { initialLibrary, loadLibrary } from '../../app/utils/library'
import { desktop } from './fixtures'
import { storedLibrary } from './libraryBackend'

it('hands documents an earlier release kept in local storage to the core, once', async () => {
  desktop()
  localStorage.setItem(
    'kchess:studies:v1',
    JSON.stringify({
      version: 1,
      items: [{ id: 'kept', name: 'Kept study', pgn: '1. e4 *', updatedAt: 1 }],
    }),
  )
  localStorage.setItem('kchess:analysis-engine', 'false')
  await loadLibrary()
  expect(initialLibrary().studies.map((s) => s.name)).toEqual(['Kept study'])
  expect(storedLibrary().imported).toBe(true)
  // Moved documents leave local storage; view preferences stay where they are.
  expect(localStorage.getItem('kchess:studies:v1')).toBeNull()
  expect(localStorage.getItem('kchess:analysis-engine')).toBe('false')
})

it('keeps local storage untouched when the core cannot take the documents', async () => {
  desktop({
    importLibrary: async () => {
      throw new Error('Disk full')
    },
  })
  const saved = JSON.stringify({ version: 1, items: [] })
  localStorage.setItem('kchess:studies:v1', saved)
  await loadLibrary()
  expect(localStorage.getItem('kchess:studies:v1')).toBe(saved)
})
