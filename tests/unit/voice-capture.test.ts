import { beforeEach, describe, expect, it, vi } from 'vitest'
import { VoiceCapture } from '../../app/utils/voiceCapture'
import { deferred } from './fixtures'

const vosk = vi.hoisted(() => ({ models: [] as FakeModel[], recognizers: [] as FakeRecognizer[] }))
type Recognition = { result: { text: string; result: { conf: number }[] } }
class FakeRecognizer {
  listeners = new Map<string, (message: Recognition) => void>()
  remove = vi.fn()
  setWords = vi.fn()
  acceptWaveformFloat = vi.fn()
  retrieveFinalResult = vi.fn()
  constructor() {
    vosk.recognizers.push(this)
  }
  on(event: string, callback: (message: Recognition) => void) {
    this.listeners.set(event, callback)
  }
  speak(text: string) {
    this.listeners.get('result')!({ result: { text, result: [{ conf: 0.95 }] } })
  }
}
class FakeModel {
  ready = true
  terminate = vi.fn()
  KaldiRecognizer = FakeRecognizer
  constructor() {
    vosk.models.push(this)
  }
  on(event: string, callback: (message: { result: boolean }) => void) {
    if (event === 'load') queueMicrotask(() => callback({ result: true }))
  }
}
vi.mock('vosk-browser', () => ({
  Model: FakeModel,
}))

class FakeContext {
  sampleRate = 16000
  destination = {}
  audioWorklet = { addModule: vi.fn(async () => {}) }
  resume = vi.fn(async () => {})
  close = vi.fn(async () => {})
  createMediaStreamSource = vi.fn(() => ({ connect: vi.fn(), disconnect: vi.fn() }))
}
class FakeNode {
  static nodes: FakeNode[] = []
  port = {
    onmessage: null as
      ((event: { data: { epoch: number; audio?: Float32Array; final?: boolean } }) => void) | null,
    postMessage: vi.fn(),
    close: vi.fn(),
  }
  connect = vi.fn()
  disconnect = vi.fn()
  constructor() {
    FakeNode.nodes.push(this)
  }
}

function setupCapture() {
  const callbacks = { state: vi.fn(), partial: vi.fn(), result: vi.fn(), error: vi.fn() }
  const track = { stop: vi.fn(), onended: null }
  const stream = { getTracks: () => [track] }
  const getUserMedia = vi.fn(async () => stream)
  Object.defineProperty(navigator, 'mediaDevices', { value: { getUserMedia }, configurable: true })
  return { capture: new VoiceCapture(callbacks), callbacks, track, stream, getUserMedia }
}

beforeEach(() => {
  vosk.models.length = 0
  vosk.recognizers.length = 0
  FakeNode.nodes.length = 0
  vi.stubGlobal('AudioContext', FakeContext)
  vi.stubGlobal('AudioWorkletNode', FakeNode)
})

describe('voice capture lifecycle', () => {
  it('drops queued results from a previous question and accepts each final utterance once', async () => {
    const { capture, callbacks } = setupCapture()
    await capture.prepare(['e four', '[unk]'])
    capture.update(true, ['e four', '[unk]'], false)
    const old = vosk.recognizers.at(-1)!
    capture.update(true, ['e four', '[unk]'], false)
    old.speak('e four')
    expect(callbacks.result).not.toHaveBeenCalled()
    const current = vosk.recognizers.at(-1)!
    current.speak('e four')
    current.speak('e four')
    expect(callbacks.result).toHaveBeenCalledTimes(1)
    expect(callbacks.result).toHaveBeenCalledWith(
      expect.objectContaining({ text: 'e four', confidence: 0.95 }),
    )
    capture.dispose()
  })

  it('closes the mic between turns, reuses the loaded model, and terminates it on disable', async () => {
    const { capture, track } = setupCapture()
    await capture.prepare(['e four'])
    capture.update(true, ['e four'], false)
    capture.update(false, ['e four'], false)
    expect(track.stop).toHaveBeenCalledOnce()
    expect(vosk.models[0]!.terminate).not.toHaveBeenCalled()
    await capture.prepare(['e four'])
    capture.update(true, ['e four'], false)
    expect(vosk.models).toHaveLength(1)
    capture.dispose()
    expect(track.stop).toHaveBeenCalledTimes(2)
    expect(vosk.models[0]!.terminate).toHaveBeenCalledOnce()
  })

  it('stops a microphone granted after the page was unmounted', async () => {
    const { capture, track, stream, getUserMedia, callbacks } = setupCapture()
    const pending = deferred<typeof stream>()
    getUserMedia.mockReturnValueOnce(pending.promise)
    const opening = capture.prepare(['e four'])
    await vi.waitFor(() => expect(getUserMedia).toHaveBeenCalled())
    capture.dispose()
    pending.resolve(stream)
    await opening
    expect(track.stop).toHaveBeenCalledOnce()
    expect(vosk.models).toHaveLength(0)
    expect(callbacks.error).not.toHaveBeenCalled()
  })

  it('reports denied permission without allocating a recognition model', async () => {
    const { capture, getUserMedia, callbacks } = setupCapture()
    getUserMedia.mockRejectedValueOnce(new DOMException('denied', 'NotAllowedError'))
    await expect(capture.prepare(['e four'])).rejects.toThrow()
    expect(callbacks.error).toHaveBeenCalledWith(
      expect.stringContaining('isn’t allowed to use the microphone'),
      'denied',
    )
    expect(vosk.models).toHaveLength(0)
    capture.dispose()
  })

  it('averages word confidence so a homophone split does not reject a clear phrase', async () => {
    const { capture, callbacks } = setupCapture()
    await capture.prepare(['knight', 'f three'])
    capture.update(true, ['knight', 'f three'], false)
    vosk.recognizers.at(-1)!.listeners.get('result')!({
      result: {
        text: 'knight f three',
        result: [
          { conf: 0.5, word: 'knight' },
          { conf: 1, word: 'f' },
          { conf: 1, word: 'three' },
          { conf: 0.2, word: '[unk]' },
        ],
      },
    } as never)
    expect(callbacks.result).toHaveBeenCalledWith({
      text: 'knight f three',
      confidence: expect.closeTo(0.833, 2),
      // Per-word confidences go to the voice history; noise markers are left out.
      words: [
        { word: 'knight', conf: 0.5 },
        { word: 'f', conf: 1 },
        { word: 'three', conf: 1 },
      ],
    })
    capture.dispose()
  })

  it('asks the OS for access first and skips getUserMedia when it is blocked', async () => {
    const { capture, getUserMedia, callbacks } = setupCapture()
    const microphoneAccess = vi.fn(async () => ({
      status: 'denied' as const,
      canOpenSettings: true,
      launchedFromTerminal: false,
    }))
    const previous = window.kchess
    window.kchess = { microphoneAccess } as unknown as typeof window.kchess
    await expect(capture.prepare(['e four'])).rejects.toThrow()
    expect(microphoneAccess).toHaveBeenCalledWith(true)
    expect(getUserMedia).not.toHaveBeenCalled()
    expect(callbacks.error).toHaveBeenCalledWith(expect.any(String), 'denied')
    window.kchess = previous
    capture.dispose()
  })

  it('explains a microphone that cannot start instead of a bare browser error', async () => {
    const { capture, getUserMedia, callbacks } = setupCapture()
    getUserMedia.mockRejectedValueOnce(
      new DOMException('Could not start audio source', 'NotReadableError'),
    )
    await expect(capture.prepare(['e four'])).rejects.toThrow()
    expect(callbacks.error).toHaveBeenCalledWith(expect.stringContaining('couldn’t start'), 'busy')
    capture.dispose()
  })

  it('accepts PCM only from the current context and drains it before a push-to-talk final result', async () => {
    const { capture } = setupCapture()
    await capture.prepare(['e four'])
    capture.update(true, ['e four'], true)
    capture.talk(true)
    const node = FakeNode.nodes.at(-1)!
    const epoch = (node.port.postMessage.mock.calls.at(-1)![0] as { epoch: number }).epoch
    const rec = vosk.recognizers.at(-1)!
    node.port.onmessage!({ data: { epoch: epoch - 1, audio: new Float32Array(128) } })
    expect(rec.acceptWaveformFloat).not.toHaveBeenCalled()
    capture.talk(false)
    expect(node.port.postMessage).toHaveBeenLastCalledWith({ action: 'flush' })
    node.port.onmessage!({ data: { epoch, audio: new Float32Array(128) } })
    node.port.onmessage!({ data: { epoch, final: true } })
    expect(rec.acceptWaveformFloat).toHaveBeenCalledOnce()
    expect(rec.retrieveFinalResult).toHaveBeenCalledOnce()
    capture.dispose()
  })
})
