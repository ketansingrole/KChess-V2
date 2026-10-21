import type {
  VoiceHeard,
  VoiceRecognized,
  VoiceWord,
  VoiceModelProgress,
} from '@kchess/contracts/types'

export type VoiceState = 'off' | 'loading' | 'paused' | 'listening' | 'error'
/** Why voice input stopped, so the UI can offer the right fix (e.g. open privacy settings). */
export type VoiceErrorKind = 'denied' | 'missing' | 'busy' | 'unsupported' | 'model' | 'other'
export interface VoiceResult {
  text: string
  confidence: number
  /** Each recognized word with its own confidence, for the voice history. */
  words?: VoiceWord[]
  /** The recognizer's text before cleanup, noise markers ([unk]) included. */
  raw?: string
  /** Below the confidence normally needed; passed on only where a control accepts it. */
  unclear?: boolean
}
interface Callbacks {
  state: (state: VoiceState) => void
  partial: (text: string) => void
  result: (result: VoiceResult) => void
  error: (message: string, kind: VoiceErrorKind) => void
  /** Input loudness 0–1 while listening, for a level meter. */
  level?: (value: number) => void
  modelProgress?: (progress: VoiceModelProgress) => void
}

class VoiceError extends Error {
  constructor(
    message: string,
    readonly kind: VoiceErrorKind,
  ) {
    super(message)
  }
}

export const MIC_DENIED_MESSAGE =
  'KChess isn’t allowed to use the microphone. Allow it in your system privacy settings, then try again.'

/** Map a getUserMedia/AudioContext failure to a message the player can act on. */
export function describeMicError(error: unknown): { message: string; kind: VoiceErrorKind } {
  if (error instanceof VoiceError) return { message: error.message, kind: error.kind }
  const name = error instanceof DOMException || error instanceof Error ? error.name : ''
  if (['NotAllowedError', 'PermissionDeniedError', 'SecurityError'].includes(name))
    return { message: MIC_DENIED_MESSAGE, kind: 'denied' }
  if (['NotFoundError', 'DevicesNotFoundError', 'OverconstrainedError'].includes(name))
    return { message: 'No microphone was found. Connect one and try again.', kind: 'missing' }
  if (['NotReadableError', 'TrackStartError', 'AbortError'].includes(name))
    return {
      message:
        'The microphone couldn’t start. Another app may be using it, or the system hasn’t granted access yet.',
      kind: 'busy',
    }
  return {
    message:
      error instanceof Error && error.message ? error.message : 'Voice input could not start.',
    kind: 'other',
  }
}

/** Ask the desktop shell for OS-level permission first; Electron won't prompt on its own. */
async function ensureMicrophonePermission(): Promise<void> {
  const access = await window.kchess?.microphoneAccess?.(true).catch((error: unknown) => {
    console.warn('[voice-capture] checking microphone permission failed:', error)
    return undefined
  })
  if (access && (access.status === 'denied' || access.status === 'restricted'))
    throw new VoiceError(MIC_DENIED_MESSAGE, 'denied')
}

export function voiceAssetUrl(file: string): string {
  return new URL(`voice/${file}`, document.baseURI).href
}

/** Microphone samples (-1…1) as base64 16-bit little-endian PCM, as the core's recognizer takes. */
export function pcmBase64(audio: Float32Array): string {
  const bytes = new Uint8Array(audio.length * 2)
  const view = new DataView(bytes.buffer)
  audio.forEach((sample, index) => {
    view.setInt16(index * 2, Math.round(Math.max(-1, Math.min(1, sample)) * 0x7fff), true)
  })
  let binary = ''
  for (let start = 0; start < bytes.length; start += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(start, start + 0x8000))
  return btoa(binary)
}

/**
 * The core has one recognizer, shared by every voice control. Their calls run one at a time on
 * one queue, so audio reaches it in order, and the control that started it last owns it: an
 * outgoing control's late audio or stop never reaches its replacement's session.
 */
let recognizerQueue: Promise<unknown> = Promise.resolve()
let recognizerOwner: object | undefined

/**
 * Microphone capture for one mounted voice control. Audio goes to the core's offline recognizer
 * (`voiceStart`/`voiceAccept`), which runs Vosk in its own process; the recognizer is reused
 * between turns/questions and released on dispose.
 */
export class VoiceCapture {
  private ready = false
  private stream?: MediaStream
  private context?: AudioContext
  private node?: AudioWorkletNode
  private source?: MediaStreamAudioSourceNode
  private revision = 0
  private epoch = 0
  private active = false
  private held = false
  private pushTalk = false
  private starting?: Promise<void>
  private grammar: readonly string[] = []
  private disposed = false
  /** This control's claim on the shared recognizer. */
  private readonly owner = {}

  constructor(private callbacks: Callbacks) {}

  /** Queues a `voiceStart`, which makes this control the recognizer's owner. */
  private claim(work: () => Promise<void>): Promise<void> {
    return this.enqueue(() => {
      recognizerOwner = this.owner
      return work()
    })
  }

  /** Queues a call on this control's session; once another control owns it, `skipped` instead. */
  private owned<T>(work: () => Promise<T>, skipped: T): Promise<T> {
    return this.enqueue(() => (recognizerOwner === this.owner ? work() : Promise.resolve(skipped)))
  }

  private enqueue<T>(work: () => Promise<T>): Promise<T> {
    const next = recognizerQueue.then(work, work)
    // The queue only orders calls: each caller handles (and logs) its own call's failure on
    // `next`, so the chain itself just waits for it to settle either way.
    const settled = () => undefined
    recognizerQueue = next.then(settled, settled)
    return next
  }

  /** Download (first use) and verify the model; the core loads it on the first `voiceStart`. */
  private async ensureModel(): Promise<void> {
    if (this.ready) return
    this.callbacks.state('loading')
    const unsubscribe = window.kchess.onVoiceModelProgress((progress) => {
      if (!this.disposed) this.callbacks.modelProgress?.(progress)
    })
    try {
      await window.kchess.ensureVoiceModel()
    } catch (cause) {
      throw new VoiceError(
        cause instanceof Error
          ? cause.message
          : 'The voice model couldn’t download. Connect to the internet and try again.',
        'model',
      )
    } finally {
      unsubscribe()
    }
    if (this.disposed) throw new Error('Voice input was stopped.')
    this.ready = true
  }

  /** Starts the core's recognizer once, which loads the model (the first start is the slow one). */
  private async warmUp(sampleRate: number): Promise<void> {
    if (!this.grammar.length) return
    const grammar = [...this.grammar]
    try {
      await this.claim(() => window.kchess.voiceStart(grammar, sampleRate))
    } catch (cause) {
      throw new VoiceError(
        cause instanceof Error && cause.message
          ? cause.message
          : 'The offline voice model couldn’t load. Try again.',
        'model',
      )
    }
  }

  /** Preflight microphone + model before a timed training run starts. */
  prepare(grammar: readonly string[]): Promise<void> {
    this.grammar = grammar
    if (this.starting)
      return this.starting.then(() => {
        if (!this.disposed && !this.context) return this.prepare(grammar)
      })
    if (this.context && this.node && this.ready) return Promise.resolve()
    const revision = this.revision
    this.starting = this.open(revision).finally(() => {
      this.starting = undefined
    })
    return this.starting
  }

  private async open(revision: number): Promise<void> {
    const started = performance.now()
    let stream: MediaStream | undefined
    let context: AudioContext | undefined
    try {
      this.callbacks.state('loading')
      if (!navigator.mediaDevices?.getUserMedia)
        throw new VoiceError('Microphone input isn’t supported in this window.', 'unsupported')
      // Request permission before waiting on the model. Stop these local tracks even if cancelled.
      await ensureMicrophonePermission()
      if (revision !== this.revision || this.disposed) return
      stream = await navigator.mediaDevices.getUserMedia({
        video: false,
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      })
      if (revision !== this.revision || this.disposed) return
      context = new AudioContext({ sampleRate: 16_000 })
      await context.resume()
      await context.audioWorklet.addModule(voiceAssetUrl('processor.js'))
      await this.ensureModel()
      if (revision !== this.revision || this.disposed) return
      // Load the model now, so a timed run never waits for it.
      await this.warmUp(context.sampleRate)
      if (revision !== this.revision || this.disposed) return
      this.stream = stream
      this.context = context
      stream = undefined
      context = undefined
      this.node = new AudioWorkletNode(this.context, 'kchess-voice', {
        numberOfInputs: 1,
        numberOfOutputs: 1,
        outputChannelCount: [1],
        channelCount: 1,
      })
      this.source = this.context.createMediaStreamSource(this.stream)
      this.source.connect(this.node)
      // The worklet emits silence, so this never monitors the microphone through the speakers.
      this.node.connect(this.context.destination)
      this.node.port.onmessage = ({
        data,
      }: MessageEvent<{ epoch: number; audio?: Float32Array; final?: boolean }>) => {
        if (data.epoch !== this.epoch || !this.active || !this.grammar.length) return
        const epoch = this.epoch
        if (data.audio) {
          this.reportLevel(data.audio)
          const audio = pcmBase64(data.audio)
          this.recognize(epoch, () => window.kchess.voiceAccept(audio))
        }
        if (data.final) this.recognize(epoch, () => window.kchess.voiceFinish())
      }
      for (const track of this.stream.getTracks()) {
        track.onended = () => this.fail('The microphone disconnected. Try again.', 'missing')
      }
      this.resetRecognizer()
      this.callbacks.state(this.active && (!this.pushTalk || this.held) ? 'listening' : 'paused')
    } catch (error) {
      if (revision === this.revision && !this.disposed) {
        const { message, kind } = describeMicError(error)
        this.fail(message, kind)
        throw error
      }
    } finally {
      void window.kchess
        ?.recordPerformance?.('voice.activation', performance.now() - started)
        .catch((error: unknown) => {
          console.warn('[voice-capture] recording activation performance failed:', error)
        })
      stream?.getTracks().forEach((track) => track.stop())
      if (context)
        await context.close().catch((error: unknown) => {
          console.warn('[voice-capture] closing audio context failed:', error)
        })
    }
  }

  /** Called synchronously when the board/question changes, invalidating queued worker results. */
  update(active: boolean, grammar: readonly string[], pushTalk: boolean): void {
    this.active = active
    this.grammar = grammar
    this.pushTalk = pushTalk
    this.held = false
    this.callbacks.partial('')
    if (!active) {
      this.pause()
      return
    }
    if (this.context) this.resetRecognizer()
    else
      void this.prepare(grammar)
        .then(() => {
          if (!this.disposed && this.active) this.resetRecognizer()
        })
        .catch((error: unknown) => {
          console.warn('[voice-capture] preparing microphone failed:', error)
        })
  }

  talk(held: boolean): void {
    if (!this.active || !this.pushTalk || !this.node || held === this.held) return
    this.held = held
    if (held) {
      this.resetRecognizer()
      this.callbacks.state('listening')
    } else {
      this.node.port.postMessage({ action: 'flush' })
      this.callbacks.level?.(0)
      this.callbacks.state('paused')
    }
  }

  /**
   * Starts a fresh utterance with the current grammar; audio of earlier epochs is dropped.
   * `continuing` (after a result) keeps this control's session without taking the recognizer
   * back from a control that claimed it meanwhile.
   */
  private resetRecognizer(continuing = false): void {
    this.epoch++
    if (!this.ready || !this.context || !this.node || !this.active) return
    const epoch = this.epoch
    if (this.grammar.length) {
      const grammar = [...this.grammar]
      const sampleRate = this.context.sampleRate
      const start = () => window.kchess.voiceStart(grammar, sampleRate)
      void (continuing ? this.owned(start, undefined) : this.claim(start)).catch(
        (error: unknown) => {
          console.warn('[voice-capture] starting speech recognition failed:', error)
          this.recognitionFailed(epoch, error)
        },
      )
    }
    this.node.port.postMessage({ action: 'reset', epoch, listening: !this.pushTalk || this.held })
    this.callbacks.state(!this.pushTalk || this.held ? 'listening' : 'paused')
  }

  /** Queues one recognizer call; its answer counts only while its epoch is current. */
  private recognize(epoch: number, call: () => Promise<VoiceHeard>): void {
    this.owned<VoiceHeard>(call, {})
      .then((heard) => {
        if (epoch !== this.epoch || !this.active) return
        if (heard.result) this.heard(epoch, heard.result)
        else if (heard.partial !== undefined) this.callbacks.partial(heard.partial)
      })
      .catch((error: unknown) => {
        console.warn('[voice-capture] speech recognition failed:', error)
        this.recognitionFailed(epoch, error)
      })
  }

  private heard(epoch: number, result: VoiceRecognized): void {
    const text = result.text.trim()
    if (!text) return
    // Judge only real words: a noise [unk] would otherwise sink a clearly heard command.
    const words: VoiceWord[] = result.words
      .filter((word) => word.word !== '[unk]')
      .map((word) => ({ word: word.word, conf: Math.round(word.conf * 1000) / 1000 }))
    const confidences = words.map((word) => word.conf)
    // Average, not minimum: grammar homophones (“to”/“two”) split a word's confidence to 0.5
    // even when heard perfectly. Moves are still checked for legality before use.
    const confidence = confidences.length
      ? confidences.reduce((sum, value) => sum + value, 0) / confidences.length
      : 0
    this.callbacks.partial('')
    this.callbacks.result({ text, confidence, words })
    // One action per finalized utterance. A parent's synchronous context update may already reset it.
    if (epoch === this.epoch) this.resetRecognizer(true)
  }

  private recognitionFailed(epoch: number, error: unknown): void {
    if (epoch !== this.epoch || this.disposed || recognizerOwner !== this.owner) return
    this.fail(
      error instanceof Error && error.message
        ? error.message
        : 'Speech recognition stopped. Try again.',
      'model',
    )
  }

  pause(): void {
    this.revision++
    this.epoch++
    this.node?.disconnect()
    this.node?.port.close()
    this.source?.disconnect()
    this.stream?.getTracks().forEach((track) => {
      track.onended = null
      track.stop()
    })
    if (this.context)
      void this.context.close().catch((error: unknown) => {
        console.warn('[voice-capture] closing audio context failed:', error)
      })
    this.node = undefined
    this.source = undefined
    this.stream = undefined
    this.context = undefined
    this.callbacks.level?.(0)
    this.callbacks.state('paused')
  }

  private reportLevel(audio: Float32Array): void {
    if (!this.callbacks.level) return
    let sum = 0
    for (const sample of audio) sum += sample * sample
    // Speech RMS rarely exceeds ~0.2; scale so normal speaking fills most of the meter.
    this.callbacks.level(Math.min(1, Math.sqrt(sum / (audio.length || 1)) * 5))
  }

  private fail(message: string, kind: VoiceErrorKind = 'other'): void {
    this.pause()
    this.callbacks.error(message, kind)
    this.callbacks.state('error')
  }

  dispose(): void {
    this.disposed = true
    this.active = false
    this.pause()
    if (this.ready)
      void this.owned(async () => {
        recognizerOwner = undefined
        await window.kchess.voiceStop()
      }, undefined).catch((error: unknown) => {
        console.warn('[voice-capture] releasing the recognizer failed:', error)
      })
    this.ready = false
    this.callbacks.state('off')
  }
}
