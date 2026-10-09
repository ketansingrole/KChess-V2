/* Original KChess microphone worklet. Sends ~100 ms mono PCM chunks; produces silence. */
class VoiceProcessor extends AudioWorkletProcessor {
  constructor() {
    super()
    this.buffer = new Float32Array(Math.round(sampleRate / 10))
    this.used = 0
    this.epoch = 0
    this.listening = false
    this.port.onmessage = ({ data }) => {
      if (data.action === 'reset') {
        this.used = 0
        this.epoch = data.epoch
        this.listening = data.listening
      } else if (data.action === 'flush') {
        this.flush()
        this.port.postMessage({ epoch: this.epoch, final: true })
        this.listening = false
      }
    }
  }
  flush() {
    if (!this.used) return
    const audio = this.buffer.slice(0, this.used)
    this.port.postMessage({ epoch: this.epoch, audio }, [audio.buffer])
    this.used = 0
  }
  process(inputs) {
    const audio = inputs[0]?.[0]
    if (!audio || !this.listening) return true
    for (const sample of audio) {
      this.buffer[this.used++] = sample
      if (this.used === this.buffer.length) this.flush()
    }
    return true
  }
}
registerProcessor('kchess-voice', VoiceProcessor)
