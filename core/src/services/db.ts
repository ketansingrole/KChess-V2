import { scopedState, platform } from './platform'
import { chmodSync } from 'node:fs'
import { join } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { logDebug } from './logger'
import { migrate } from './migrations'

export function dbPath(): string {
  return join(platform().dataDir, 'kchess.db')
}

export function getDb(): DatabaseSync {
  platform() // Reject late work from a closed host before touching a new profile.
  if (serviceState.db) return serviceState.db
  const path = dbPath()
  const instance = new DatabaseSync(path)
  instance.exec('PRAGMA journal_mode = WAL;')
  instance.exec('PRAGMA foreign_keys = ON;')
  // WAL makes NORMAL durable enough for a local cache-like store and avoids an fsync per commit.
  instance.exec('PRAGMA synchronous = NORMAL;')
  instance.exec('PRAGMA temp_store = MEMORY;')
  instance.exec('PRAGMA cache_size = -20000;')
  migrate(instance)
  try {
    chmodSync(path, 0o600)
  } catch (cause) {
    logDebug('db', 'chmod failed:', cause)
  }
  serviceState.db = instance
  return serviceState.db
}

export function closeDb(): void {
  try {
    serviceState.db?.close()
  } catch (cause) {
    logDebug('db', 'Close failed:', cause)
  }
  serviceState.db = null
}

const serviceState = scopedState(() => ({
  db: null as DatabaseSync | null,
}))
