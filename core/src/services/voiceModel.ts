import { createHash } from 'node:crypto'
import { createReadStream, createWriteStream } from 'node:fs'
import { mkdir, mkdtemp, readFile, rename, rm, stat, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { Worker } from 'node:worker_threads'
import { Readable, Transform } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import { logDebug } from './logger.ts'
import type { VoiceModelProgress, VoiceModelStatus } from '../contracts/types'

export const VOICE_MODEL = {
  name: 'vosk-model-small-en-us-0.15',
  url: 'https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip',
  sha256: '30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498',
}
const MAX_DOWNLOAD = 50 * 1024 * 1024

async function checksum(path: string): Promise<string> {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return hash.digest('hex')
}

interface PreparedArchive {
  prepared: string
  marker: { source: string; sha256: string; size: number }
}
async function prepareArchive(
  archive: string,
  directory: string,
  model: typeof VOICE_MODEL,
): Promise<PreparedArchive> {
  const worker = new Worker(
    new URL(
      import.meta.url.endsWith('.ts') ? './voiceModelWorker.ts' : './voiceModelWorker.js',
      import.meta.url,
    ),
    { workerData: { archive, directory, model } },
  )
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    return await new Promise<PreparedArchive>((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('Voice model preparation timed out.')), 120_000)
      worker.once('message', (message: PreparedArchive & { error?: string }) => {
        if (message.error) reject(new Error(message.error))
        else resolve(message)
      })
      worker.once('error', reject)
      worker.once('exit', () => reject(new Error('Voice model preparation stopped unexpectedly.')))
    })
  } finally {
    clearTimeout(timer)
    await worker.terminate()
  }
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

  async status(): Promise<VoiceModelStatus> {
    if (this.pending) return { installed: false, bytes: 0, busy: true, progress: this.progress }
    const installed = await this.cached()
    // Preparation may start while the cache checksum is being checked.
    if (this.pending) return { installed: false, bytes: 0, busy: true, progress: this.progress }
    if (!installed) return { installed: false, bytes: 0, busy: false }
    const bytes = await stat(this.path)
      .then((file) => file.size)
      .catch((cause: unknown) => {
        logDebug('voice', 'Voice model file is missing:', cause)
        return 0
      })
    return { installed: bytes > 0, bytes, busy: false }
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
    } catch (cause) {
      logDebug('voice', 'Voice model cache check failed:', cause)
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
      const archive = join(staging, 'model.zip')
      let received = 0
      await pipeline(
        Readable.fromWeb(response.body as never),
        new Transform({
          transform: (chunk: Buffer, _encoding, done) => {
            received += chunk.byteLength
            if (received > MAX_DOWNLOAD)
              return done(new Error('Voice model download is too large.'))
            this.report({ phase: 'downloading', received, total })
            done(null, chunk)
          },
        }),
        createWriteStream(archive),
      )
      this.report({ phase: 'preparing', received })
      const { marker, prepared } = await prepareArchive(archive, staging, this.model)
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
