import { afterAll, describe, expect, it } from 'vitest'
import { closeNativeCore, nativeCall } from '@kchess/native/nativeCore'
import { fakeSecrets, useTestPlatform } from '../fixtures/corePlatform'

/** The Rust core asks the host for capabilities and gets the platform's answers back. */
describe('host capabilities through the native core', () => {
  const opened: string[] = []
  useTestPlatform({
    secrets: fakeSecrets(true),
    openExternal: async (url) => void opened.push(url),
    onBattery: () => true,
  })
  afterAll(() => closeNativeCore())

  it('answers secrets, the browser and power state', async () => {
    expect(await nativeCall('host.ask', 'secrets.available')).toBe(true)
    const sealed = await nativeCall<string>('host.ask', 'secrets.encrypt', 'lip_token')
    expect(sealed).not.toBe('lip_token')
    expect(await nativeCall('host.ask', 'secrets.decrypt', sealed)).toBe('lip_token')
    expect(await nativeCall('host.ask', 'onBattery')).toBe(true)
    await nativeCall('host.ask', 'openExternal', 'https://lichess.org/oauth')
    expect(opened).toEqual(['https://lichess.org/oauth'])
  })

  it('reports a capability the host does not have as an error', async () => {
    await expect(nativeCall('host.ask', 'teleport')).rejects.toThrow(
      'Unknown host capability teleport.',
    )
  })
})
