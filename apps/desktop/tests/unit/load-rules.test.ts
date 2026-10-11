import { describe, expect, it, vi } from 'vitest'
import { instantiateRules } from '../../app/utils/loadRules'

const emptyModule = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])

describe('rules module loading', () => {
  it('streams a WASM response without buffering its source in JavaScript', async () => {
    const response = new Response(emptyModule, { headers: { 'content-type': 'application/wasm' } })
    const buffer = vi.spyOn(response, 'arrayBuffer')
    // happy-dom's Response is not Node's native Response accepted by instantiateStreaming.
    const streaming = vi
      .spyOn(WebAssembly, 'instantiateStreaming')
      .mockImplementation(async () => WebAssembly.instantiate(emptyModule))
    expect(await instantiateRules(response)).toBeInstanceOf(WebAssembly.Instance)
    expect(streaming).toHaveBeenCalledWith(response)
    expect(buffer).not.toHaveBeenCalled()
  })

  it('supports hosts without a WASM content type', async () => {
    const response = new Response(emptyModule)
    const buffer = vi.spyOn(response, 'arrayBuffer')
    expect(await instantiateRules(response)).toBeInstanceOf(WebAssembly.Instance)
    expect(buffer).toHaveBeenCalledOnce()
  })

  it('preserves HTTP and compilation failures', async () => {
    await expect(instantiateRules(new Response('', { status: 404 }))).rejects.toThrow('404')
    await expect(instantiateRules(new Response('invalid'))).rejects.toThrow()
  })
})
