import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { mkdir, mkdtemp, readFile, rename, rm, stat, writeFile } from 'node:fs/promises'
import { dirname, join, resolve, sep } from 'node:path'
import { unzipSync } from 'fflate'
import { create } from 'tar'
import type { VoiceModelProgress } from '../shared/types'

export const VOICE_MODEL = {
  name: 'vosk-model-small-en-us-0.15',
  url: 'https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip',
  sha256: '30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498',
}
const MAX_DOWNLOAD = 50 * 1024 * 1024
const MAX_EXTRACTED = 200 * 1024 * 1024

async function checksum(path: string): Promise<string> {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return hash.digest('hex')
}

/** One verified, persistent model per profile. Concurrent voice controls share preparation. */
export class VoiceModelCache {
  readonly path: string
  private pending?: Promise<string>
  private listeners = new Set<(progress: VoiceModelProgress) => void>()
  private progress?: VoiceModelProgress
  private directory: string
  private fetchModel: typeof fetch
  private model: typeof VOICE_MODEL

  constructor(directory: string, fetchModel: typeof fetch = fetch, model = VOICE_MODEL) {
    this.directory = directory
    this.fetchModel = fetchModel
    this.model = model
    this.path = join(directory, 'model.tar.gz')
  }

  ensure(listener: (progress: VoiceModelProgress) => void = () => {}): Promise<string> {
    this.listeners.add(listener)
    if (this.progress) listener(this.progress)
    this.pending ??= this.prepare().finally(() => {
      this.pending = undefined
      this.progress = undefined
    })
    return this.pending.finally(() => this.listeners.delete(listener))
  }

  private report(progress: VoiceModelProgress): void {
    this.progress = progress
    for (const listener of this.listeners) listener(progress)
  }

  private async cached(): Promise<boolean> {
    try {
      const marker = JSON.parse(await readFile(`${this.path}.json`, 'utf8')) as {
        source: string
        sha256: string
        size: number
      }
      return (
        marker.source === this.model.sha256 &&
        marker.size > 0 &&
        (await stat(this.path)).size === marker.size &&
        (await checksum(this.path)) === marker.sha256
      )
    } catch {
      return false
    }
  }

  private async prepare(): Promise<string> {
    this.report({ phase: 'checking', received: 0 })
    if (await this.cached()) return this.path
    await mkdir(this.directory, { recursive: true })
    const staging = await mkdtemp(join(this.directory, '.prepare-'))
    try {
      this.report({ phase: 'downloading', received: 0 })
      const response = await this.fetchModel(this.model.url, {
        signal: AbortSignal.timeout(300_000),
      })
      if (!response.ok || !response.body)
        throw new Error(`Voice model download failed: HTTP ${response.status}.`)
      const total = Number(response.headers.get('content-length')) || undefined
      if (total && total > MAX_DOWNLOAD) throw new Error('Voice model download is too large.')
      const chunks: Uint8Array[] = []
      let received = 0
      const reader = response.body.getReader()
      try {
        for (;;) {
          const { value, done } = await reader.read()
          if (done) break
          received += value.byteLength
          if (received > MAX_DOWNLOAD) throw new Error('Voice model download is too large.')
          chunks.push(value)
          this.report({ phase: 'downloading', received, total })
        }
      } finally {
        await reader.cancel().catch(() => {})
        reader.releaseLock()
      }
      const zip = Buffer.concat(chunks)
      if (createHash('sha256').update(zip).digest('hex') !== this.model.sha256)
        throw new Error('Voice model checksum mismatch. Try downloading it again.')
      this.report({ phase: 'preparing', received })
      const extracted = join(staging, 'unpacked')
      let bytes = 0
      const files = unzipSync(zip, {
        filter: (entry) => {
          bytes += entry.originalSize
          const path = resolve(extracted, entry.name)
          if (
            bytes > MAX_EXTRACTED ||
            !entry.name.startsWith(`${this.model.name}/`) ||
            !path.startsWith(`${resolve(extracted, this.model.name)}${sep}`) ||
            entry.name.includes('\\')
          ) {
            // ZIPs contain a directory entry for the model's root.
            if (entry.name === `${this.model.name}/` && !entry.originalSize) return false
            throw new Error('Invalid voice model archive.')
          }
          return !entry.name.endsWith('/')
        },
      })
      if (!Object.keys(files).length) throw new Error('Voice model archive is empty.')
      for (const [path, data] of Object.entries(files)) {
        const target = resolve(extracted, path)
        await mkdir(dirname(target), { recursive: true })
        await writeFile(target, data)
      }
      const prepared = join(staging, 'model.tar.gz')
      await create({ cwd: extracted, file: prepared, gzip: true, portable: true }, [
        this.model.name,
      ])
      const marker = {
        source: this.model.sha256,
        sha256: await checksum(prepared),
        size: (await stat(prepared)).size,
      }
      // Only publish a fully prepared archive; a missing marker makes interrupted installs retry.
      await rm(`${this.path}.json`, { force: true })
      await rm(this.path, { force: true })
      await rename(prepared, this.path)
      const ready = join(staging, 'ready.json')
      await writeFile(ready, JSON.stringify(marker))
      await rename(ready, `${this.path}.json`)
      return this.path
    } catch (cause) {
      const detail = cause instanceof Error ? cause.message : 'Download failed.'
      throw new Error(
        `Couldn’t prepare voice input. ${detail} Connect to the internet and try again.`,
        { cause },
      )
    } finally {
      await rm(staging, { recursive: true, force: true })
    }
  }
}
