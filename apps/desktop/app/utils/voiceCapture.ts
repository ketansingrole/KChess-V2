import type { Model, KaldiRecognizer } from 'vosk-browser/dist/model'
import type { VoiceWord, VoiceModelProgress } from '../../../../core/src/contracts/types'

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

/** One model per mounted voice control, reused between turns/questions; released on disable. */
export class VoiceCapture {
  private model?: Model
  private loading?: Promise<Model>
  private stream?: MediaStream
  private context?: AudioContext
  private node?: AudioWorkletNode
  private source?: MediaStreamAudioSourceNode
  private recognizer?: KaldiRecognizer
  private revision = 0
  private epoch = 0
  private active = false
  private held = false
  private pushTalk = false
  private starting?: Promise<void>
  private grammar: readonly string[] = []
  private disposed = false
  private cancelLoad?: () => void

  constructor(private callbacks: Callbacks) {}

  private async loadModel(): Promise<Model> {
    if (this.model?.ready) return this.model
    if (this.loading) return this.loading
    this.callbacks.state('loading')
    this.loading = (async () => {
      // The package embeds Vosk's WASM in a worker. Import it only after an explicit enable action.
      const unsubscribe = window.kchess.onVoiceModelProgress((progress) => {
        if (!this.disposed) this.callbacks.modelProgress?.(progress)
      })
      let modelUrl: string
      try {
        modelUrl = await window.kchess.ensureVoiceModel()
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
      const { Model } = await import('vosk-browser')
      if (this.disposed) throw new Error('Voice input was stopped.')
      const model = new Model(modelUrl, -1)
      this.model = model
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(
          () =>
            reject(new VoiceError('The voice model took too long to load. Try again.', 'model')),
          60_000,
        )
        this.cancelLoad = () => {
          clearTimeout(timer)
          reject(new Error('Voice input was stopped.'))
        }
        model.on('load', (message) => {
          clearTimeout(timer)
          if ('result' in message && message.result) resolve()
          else reject(new VoiceError('The offline voice model couldn’t load.', 'model'))
        })
        model.on('error', (_message) => {
          clearTimeout(timer)
          reject(new VoiceError('The offline voice model couldn’t load. Try again.', 'model'))
        })
      })
      this.cancelLoad = undefined
      return model
    })()
    try {
      return await this.loading
    } finally {
      this.loading = undefined
    }
  }

  /** Preflight microphone + model before a timed training run starts. */
  prepare(grammar: readonly string[]): Promise<void> {
    this.grammar = grammar
    if (this.starting)
      return this.starting.then(() => {
        if (!this.disposed && !this.context) return this.prepare(grammar)
      })
    if (this.context && this.node && this.model?.ready) return Promise.resolve()
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
      const model = await this.loadModel()
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
        if (data.epoch !== this.epoch || !this.active || !this.recognizer) return
        if (data.audio) {
          this.recognizer.acceptWaveformFloat(data.audio, this.context!.sampleRate)
          this.reportLevel(data.audio)
        }
        if (data.final) this.recognizer.retrieveFinalResult()
      }
      for (const track of this.stream.getTracks()) {
        track.onended = () => this.fail('The microphone disconnected. Try again.', 'missing')
      }
      if (!model.ready) throw new VoiceError('The voice model is unavailable.', 'model')
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

  private resetRecognizer(): void {
    this.epoch++
    this.recognizer?.remove()
    this.recognizer = undefined
    if (!this.model?.ready || !this.context || !this.node || !this.active) return
    const epoch = this.epoch
    const rec = new this.model.KaldiRecognizer(
      this.context.sampleRate,
      JSON.stringify(this.grammar),
    )
    this.recognizer = rec
    rec.setWords(true)
    rec.on('partialresult', (message) => {
      if (epoch === this.epoch && this.active && 'result' in message && 'partial' in message.result)
        this.callbacks.partial(message.result.partial)
    })
    rec.on('result', (message) => {
      if (
        epoch !== this.epoch ||
        !this.active ||
        !('result' in message) ||
        !('text' in message.result)
      )
        return
      const text = message.result.text.trim()
      if (!text) return
      // Judge only real words: a noise [unk] would otherwise sink a clearly heard command.
      const words: VoiceWord[] =
        message.result.result
          ?.filter((word) => word.word !== '[unk]')
          .map((word) => ({ word: word.word, conf: Math.round(word.conf * 1000) / 1000 })) ?? []
      const confidences = words.map((word) => word.conf)
      // Average, not minimum: grammar homophones (“to”/“two”) split a word's confidence to 0.5
      // even when heard perfectly. Moves are still checked for legality before use.
      const confidence = confidences.length
        ? confidences.reduce((sum, value) => sum + value, 0) / confidences.length
        : 0
      this.callbacks.partial('')
      this.callbacks.result({ text, confidence, words })
      // One action per finalized utterance. A parent's synchronous context update may already reset it.
      if (epoch === this.epoch) this.resetRecognizer()
    })
    rec.on('error', () => {
      if (epoch === this.epoch) this.fail('Speech recognition stopped. Try again.', 'model')
    })
    this.node.port.postMessage({ action: 'reset', epoch, listening: !this.pushTalk || this.held })
    this.callbacks.state(!this.pushTalk || this.held ? 'listening' : 'paused')
  }

  pause(): void {
    this.revision++
    this.epoch++
    this.recognizer?.remove()
    this.recognizer = undefined
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
    this.cancelLoad?.()
    this.model?.terminate()
    this.model = undefined
    this.callbacks.state('off')
  }
}
