import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { VoiceHeard } from '@kchess/contracts/types'
import { VoiceCapture, pcmBase64 } from '../../app/utils/voiceCapture'
import { deferred } from './fixtures'

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

/** The core's recognizer: answers come from `heard`, one per `voiceAccept`/`voiceFinish`. */
function fakeRecognizer() {
  const heard: Promise<VoiceHeard>[] = []
  const answer = () => heard.shift() ?? Promise.resolve({ partial: '' })
  return {
    heard,
    voiceStart: vi.fn(async (_grammar: string[], _sampleRate: number) => {}),
    voiceAccept: vi.fn((_audio: string) => answer()),
    voiceFinish: vi.fn(() => answer()),
    voiceStop: vi.fn(async () => {}),
  }
}
let core: ReturnType<typeof fakeRecognizer>

function setupCapture() {
  const callbacks = { state: vi.fn(), partial: vi.fn(), result: vi.fn(), error: vi.fn() }
  const track = { stop: vi.fn(), onended: null }
  const stream = { getTracks: () => [track] }
  const getUserMedia = vi.fn(async () => stream)
  Object.defineProperty(navigator, 'mediaDevices', { value: { getUserMedia }, configurable: true })
  return { capture: new VoiceCapture(callbacks), callbacks, track, stream, getUserMedia }
}

/** The worklet's current epoch, from its last `reset`. */
function epochOf(node: FakeNode): number {
  return (node.port.postMessage.mock.calls.at(-1)![0] as { epoch: number }).epoch
}

function said(text: string, words: [string, number][]): VoiceHeard {
  return { result: { text, words: words.map(([word, conf]) => ({ word, conf })) } }
}

/** Lets queued recognizer calls and their answers run. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

beforeEach(() => {
  FakeNode.nodes.length = 0
  vi.stubGlobal('AudioContext', FakeContext)
  vi.stubGlobal('AudioWorkletNode', FakeNode)
  core = fakeRecognizer()
  // Keep the setup's library API, which the app facade saves through on teardown.
  window.kchess = {
    ...window.kchess,
    ensureVoiceModel: vi.fn(async () => {}),
    onVoiceModelProgress: vi.fn(() => vi.fn()),
    ...core,
  } as unknown as typeof window.kchess
})

describe('voice capture lifecycle', () => {
  it('prepares the cached model and loads the recognizer before a run starts', async () => {
    const { capture } = setupCapture()
    await capture.prepare(['e four'])
    expect(window.kchess.ensureVoiceModel).toHaveBeenCalledOnce()
    expect(core.voiceStart).toHaveBeenCalledExactlyOnceWith(['e four'], 16000)
    capture.update(true, ['e four'], false)
    await settle()
    expect(core.voiceStart).toHaveBeenCalledTimes(2)
    capture.dispose()
  })

  it('reports a model the recognizer cannot load and closes the microphone', async () => {
    const { capture, callbacks, track } = setupCapture()
    core.voiceStart.mockRejectedValueOnce(
      new Error('The offline voice model couldn’t load. Try again.'),
    )
    await expect(capture.prepare(['e four'])).rejects.toThrow('couldn’t load')
    expect(callbacks.error).toHaveBeenCalledWith(
      'The offline voice model couldn’t load. Try again.',
      'model',
    )
    expect(track.stop).toHaveBeenCalledOnce()
    capture.dispose()
  })

  it('reports download failures as retryable model errors and closes the microphone', async () => {
    const { capture, callbacks, track } = setupCapture()
    vi.mocked(window.kchess.ensureVoiceModel).mockRejectedValueOnce(
      new Error('Connect to the internet and try again.'),
    )
    await expect(capture.prepare(['e four'])).rejects.toThrow('Connect to the internet')
    expect(callbacks.error).toHaveBeenCalledWith(
      expect.stringContaining('Connect to the internet'),
      'model',
    )
    expect(track.stop).toHaveBeenCalledOnce()
    await capture.prepare(['e four'])
    expect(window.kchess.ensureVoiceModel).toHaveBeenCalledTimes(2)
    capture.dispose()
  })

  it('does not start recognition after being disabled during download', async () => {
    const { capture, callbacks, track } = setupCapture()
    const download = deferred<undefined>()
    vi.mocked(window.kchess.ensureVoiceModel).mockReturnValueOnce(download.promise)
    const opening = capture.prepare(['e four'])
    await vi.waitFor(() => expect(window.kchess.ensureVoiceModel).toHaveBeenCalled())
    capture.dispose()
    download.resolve(undefined)
    await opening
    await settle()
    expect(core.voiceStart).not.toHaveBeenCalled()
    expect(core.voiceStop).not.toHaveBeenCalled()
    expect(track.stop).toHaveBeenCalledOnce()
    expect(callbacks.error).not.toHaveBeenCalled()
  })

  it('drops answers from a previous question and accepts each final utterance once', async () => {
    const { capture, callbacks } = setupCapture()
    await capture.prepare(['e four', '[unk]'])
    capture.update(true, ['e four', '[unk]'], false)
    const node = FakeNode.nodes.at(-1)!
    // An answer still in flight when the question changes is dropped.
    const late = deferred<VoiceHeard>()
    core.heard.push(late.promise)
    node.port.onmessage!({ data: { epoch: epochOf(node), audio: new Float32Array(8) } })
    capture.update(true, ['e four', '[unk]'], false)
    late.resolve(
      said('e four', [
        ['e', 0.95],
        ['four', 0.95],
      ]),
    )
    await settle()
    expect(callbacks.result).not.toHaveBeenCalled()

    // Two chunks of the same utterance: the first result resets, so the second is not used.
    const epoch = epochOf(node)
    core.heard.push(
      Promise.resolve(
        said('e four', [
          ['e', 0.95],
          ['four', 0.95],
        ]),
      ),
      Promise.resolve(
        said('e four', [
          ['e', 0.95],
          ['four', 0.95],
        ]),
      ),
    )
    node.port.onmessage!({ data: { epoch, audio: new Float32Array(8) } })
    node.port.onmessage!({ data: { epoch, audio: new Float32Array(8) } })
    await settle()
    expect(callbacks.result).toHaveBeenCalledTimes(1)
    expect(callbacks.result).toHaveBeenCalledWith(
      expect.objectContaining({ text: 'e four', confidence: 0.95 }),
    )
    capture.dispose()
  })

  // Each control had its own queue to the one shared recognizer: a replaced control's stop,
  // queued behind its pending audio, killed the replacement's session.
  it('never lets a replaced control stop its replacement’s recognizer', async () => {
    const old = setupCapture()
    await old.capture.prepare(['e four'])
    old.capture.update(true, ['e four'], false)
    const oldNode = FakeNode.nodes.at(-1)!
    const pending = deferred<VoiceHeard>()
    core.heard.push(pending.promise)
    oldNode.port.onmessage!({ data: { epoch: epochOf(oldNode), audio: new Float32Array(8) } })

    // The replacement prepares (its start waits behind the old audio), then the old one goes.
    const replacement = setupCapture()
    const ready = replacement.capture.prepare(['d four'])
    await settle()
    old.capture.dispose()
    pending.resolve({ partial: '' })
    await ready
    await settle()
    expect(core.voiceStart.mock.lastCall![0]).toEqual(['d four'])
    expect(core.voiceStop).not.toHaveBeenCalled()

    replacement.capture.dispose()
    await settle()
    expect(core.voiceStop).toHaveBeenCalledTimes(1)
  })

  it('closes the mic between turns, reuses the recognizer, and releases it on disable', async () => {
    const { capture, track } = setupCapture()
    await capture.prepare(['e four'])
    capture.update(true, ['e four'], false)
    capture.update(false, ['e four'], false)
    expect(track.stop).toHaveBeenCalledOnce()
    await capture.prepare(['e four'])
    capture.update(true, ['e four'], false)
    await settle()
    expect(window.kchess.ensureVoiceModel).toHaveBeenCalledOnce()
    expect(core.voiceStop).not.toHaveBeenCalled()
    capture.dispose()
    await settle()
    expect(track.stop).toHaveBeenCalledTimes(2)
    expect(core.voiceStop).toHaveBeenCalledOnce()
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
    expect(window.kchess.ensureVoiceModel).not.toHaveBeenCalled()
    expect(callbacks.error).not.toHaveBeenCalled()
  })

  it('reports denied permission without preparing the model', async () => {
    const { capture, getUserMedia, callbacks } = setupCapture()
    getUserMedia.mockRejectedValueOnce(new DOMException('denied', 'NotAllowedError'))
    await expect(capture.prepare(['e four'])).rejects.toThrow()
    expect(callbacks.error).toHaveBeenCalledWith(
      expect.stringContaining('isn’t allowed to use the microphone'),
      'denied',
    )
    expect(window.kchess.ensureVoiceModel).not.toHaveBeenCalled()
    capture.dispose()
  })

  it('averages word confidence so a homophone split does not reject a clear phrase', async () => {
    const { capture, callbacks } = setupCapture()
    await capture.prepare(['knight', 'f three'])
    capture.update(true, ['knight', 'f three'], false)
    const node = FakeNode.nodes.at(-1)!
    core.heard.push(
      Promise.resolve(
        said('knight f three', [
          ['knight', 0.5],
          ['f', 1],
          ['three', 1],
          ['[unk]', 0.2],
        ]),
      ),
    )
    node.port.onmessage!({ data: { epoch: epochOf(node), audio: new Float32Array(8) } })
    await settle()
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

  it('shows partial words and reports a stopped recognizer with the core’s message', async () => {
    const { capture, callbacks } = setupCapture()
    await capture.prepare(['e four'])
    capture.update(true, ['e four'], false)
    const node = FakeNode.nodes.at(-1)!
    core.heard.push(
      Promise.resolve({ partial: 'e' }),
      Promise.reject(new Error('Speech recognition stopped. Try again.')),
    )
    node.port.onmessage!({ data: { epoch: epochOf(node), audio: new Float32Array(8) } })
    node.port.onmessage!({ data: { epoch: epochOf(node), audio: new Float32Array(8) } })
    await settle()
    expect(callbacks.partial).toHaveBeenCalledWith('e')
    expect(callbacks.error).toHaveBeenCalledWith('Speech recognition stopped. Try again.', 'model')
    expect(callbacks.state).toHaveBeenLastCalledWith('error')
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

  it('sends PCM only from the current context and drains it before a push-to-talk final result', async () => {
    const { capture } = setupCapture()
    await capture.prepare(['e four'])
    capture.update(true, ['e four'], true)
    capture.talk(true)
    const node = FakeNode.nodes.at(-1)!
    const epoch = epochOf(node)
    node.port.onmessage!({ data: { epoch: epoch - 1, audio: new Float32Array(128) } })
    expect(core.voiceAccept).not.toHaveBeenCalled()
    capture.talk(false)
    expect(node.port.postMessage).toHaveBeenLastCalledWith({ action: 'flush' })
    node.port.onmessage!({ data: { epoch, audio: new Float32Array(128) } })
    node.port.onmessage!({ data: { epoch, final: true } })
    await settle()
    expect(core.voiceAccept).toHaveBeenCalledExactlyOnceWith(pcmBase64(new Float32Array(128)))
    expect(core.voiceFinish).toHaveBeenCalledOnce()
    // In order: the grammar, then the audio, then the end of the utterance.
    const [start] = core.voiceStart.mock.invocationCallOrder.slice(-1)
    const [accept] = core.voiceAccept.mock.invocationCallOrder
    const [finish] = core.voiceFinish.mock.invocationCallOrder
    expect(start! < accept! && accept! < finish!).toBe(true)
    capture.dispose()
  })

  it('encodes samples as clamped 16-bit little-endian PCM', () => {
    const encoded = pcmBase64(new Float32Array([0, 1, -1, 0.5, 2]))
    const bytes = Uint8Array.from(atob(encoded), (char) => char.charCodeAt(0))
    expect([...new Int16Array(bytes.buffer)]).toEqual([0, 32767, -32767, 16384, 32767])
  })
})
