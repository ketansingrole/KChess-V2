import captureUrl from '../assets/sounds/Capture.mp3'
import lowTimeUrl from '../assets/sounds/LowTime.mp3'
import notifyUrl from '../assets/sounds/GenericNotify.mp3'
import moveUrl from '../assets/sounds/Move.mp3'

export type SoundName = 'move' | 'capture' | 'lowTime' | 'genericNotify'

const paths: Record<SoundName, string> = {
  move: moveUrl,
  capture: captureUrl,
  lowTime: lowTimeUrl,
  genericNotify: notifyUrl,
}

const buffers = new Map<SoundName, AudioBuffer>()
let context: AudioContext | undefined
let enabled = true
let volume = 1

/** Matches lila's `site.sound.move()`: capture sound wins, otherwise move. */
export function playMoveSound(san?: string): void {
  void play(san?.includes('x') ? 'capture' : 'move')
}

export function configure(options: { enabled: boolean; volume: number }): void {
  enabled = options.enabled
  volume = Math.max(0, Math.min(1, options.volume))
}

export async function play(name: SoundName, gain = 1): Promise<void> {
  if (!enabled) return
  try {
    const ctx = await audioContext()
    const buffer = buffers.get(name) ?? (await load(ctx, name))
    if (!buffer || ctx.state !== 'running') return
    const source = ctx.createBufferSource()
    const node = ctx.createGain()
    source.buffer = buffer
    node.gain.value = volume * gain
    source.connect(node).connect(ctx.destination)
    source.start()
  } catch (cause) {
    console.warn('[sound] playing sound failed:', cause)
    /* sound is optional */
  }
}

async function load(ctx: AudioContext, name: SoundName): Promise<AudioBuffer | undefined> {
  const response = await fetch(paths[name])
  if (!response.ok) return undefined
  const buffer = await ctx.decodeAudioData(await response.arrayBuffer())
  buffers.set(name, buffer)
  return buffer
}

// Lichess primes the audio context on the first user gesture (autoplay policy).
const primerEvents = ['pointerdown', 'pointerup', 'keydown'] as const
let primed = false

function prime(): void {
  primed = true
  for (const event of primerEvents) window.removeEventListener(event, prime, { capture: true })
  if (context?.state === 'suspended')
    void context.resume().catch((error: unknown) => {
      console.warn('[sound] resuming audio context failed:', error)
    })
}

function audioContext(): Promise<AudioContext> {
  if (context && context.state !== 'closed') return Promise.resolve(context)
  context = new AudioContext()
  if (!primed)
    for (const event of primerEvents) window.addEventListener(event, prime, { capture: true })
  return Promise.resolve(context)
}
