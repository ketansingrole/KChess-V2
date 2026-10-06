import { chmodSync } from 'node:fs'
import { join } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { app } from 'electron'
import { logDebug } from './logger'
import { migrate } from './migrations'

let db: DatabaseSync | null = null

export function dbPath(): string {
  return join(app.getPath('userData'), 'kchess.db')
}

export function getDb(): DatabaseSync {
  if (db) return db
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
  db = instance
  return db
}

export function closeDb(): void {
  try {
    db?.close()
  } catch (cause) {
    logDebug('db', 'Close failed:', cause)
  }
  db = null
}
