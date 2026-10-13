import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, it } from 'vitest'
import type { CoreEvents } from '@kchess/contracts/core'

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..', '..')
const workerSource = readFileSync(join(root, 'hosts/node/src/worker.ts'), 'utf8')

function workerEvents(): string[] {
  const block = workerSource.split('const events = [')[1]?.split('] as const')[0] ?? ''
  return [...block.matchAll(/'([^']+)'/g)].map((match) => match[1])
}

it('forwards every CoreEvents variant to Node clients', () => {
  const forwarded = new Set(workerEvents())
  // Compile-time keys of the generated `CoreEvents` map.
  const names = [
    'online:state',
    'online:event',
    'online:error',
    'puzzledb:progress',
    'engine:analysis',
    'review:update',
    'review:status',
    'challenges:update',
    'online:lobby',
    'online:ongoing-changed',
    'watch:state',
    'watch:frame',
    'watch:broadcast',
    'challenge:received',
    'settings:saved',
  ] satisfies (keyof CoreEvents)[]
  expect(forwarded.size).toBe(names.length)
  for (const name of names) {
    expect(forwarded.has(name), `worker forwards ${name}`).toBe(true)
  }
})
