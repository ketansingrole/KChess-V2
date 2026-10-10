import { EventEmitter } from 'node:events'
import { describe, expect, it, vi } from 'vitest'
import { AppUpdates, type UpdatePreferences } from '../../electron/main/appUpdates'
import { useAppUpdatesStore } from '../../app/stores/appUpdates'
import { DEFAULT_SETTINGS } from '@kchess/contracts/defaultSettings'
import type { AppUpdateStatus } from '@kchess/contracts/types'
import { deferred, desktop } from './fixtures'

const capabilities = { currentVersion: '2026.10.0', canCheck: true, canInstall: true }
const release = { version: '2026.10.1', releaseDate: '2026-10-03T00:00:00.000Z' }

/** Models the updater's synchronous events and mutable saved preferences. */
function fixture(
  options: { canCheck?: boolean; canInstall?: boolean; settings?: Partial<UpdatePreferences> } = {},
) {
  let settings = { ...DEFAULT_SETTINGS, ...options.settings }
  const updater = Object.assign(new EventEmitter(), {
    autoDownload: true,
    autoInstallOnAppQuit: true,
    autoRunAppAfterInstall: true,
    allowPrerelease: true,
    allowDowngrade: true,
    disableWebInstaller: false,
    checkForUpdates: vi.fn(async () => {
      updater.emit('update-available', release)
      return { updateInfo: release }
    }),
    downloadUpdate: vi.fn(async () => {
      updater.emit('download-progress', {
        percent: 50,
        transferred: 50,
        total: 100,
        bytesPerSecond: 10,
      })
      updater.emit('update-downloaded', release)
      return ['/verified-installer']
    }),
    quitAndInstall: vi.fn(),
  })
  const emit = vi.fn()
  const readPreferences = vi.fn(async () => settings)
  const onInstallFailure = vi.fn()
  const updates = new AppUpdates(
    updater as unknown as ConstructorParameters<typeof AppUpdates>[0],
    { ...capabilities, canCheck: options.canCheck ?? true, canInstall: options.canInstall ?? true },
    readPreferences,
    emit,
    onInstallFailure,
  )
  updates.applyPreferences(settings)
  return {
    updater,
    updates,
    emit,
    readPreferences,
    onInstallFailure,
    preferences: (patch: Partial<UpdatePreferences>) => {
      settings = { ...settings, ...patch }
      updates.applyPreferences(settings)
    },
  }
}

describe('desktop updater', () => {
  it('downloads automatically, exposes progress, and waits for a user quit without restarting', async () => {
    const { updates, updater, emit } = fixture()
    await updates.check()
    await updates.download()
    expect(updater.autoDownload).toBe(false)
    expect(updater.autoInstallOnAppQuit).toBe(false)
    expect(updater.allowPrerelease).toBe(false)
    expect(updater.allowDowngrade).toBe(false)
    expect(updater.disableWebInstaller).toBe(true)
    expect(emit.mock.calls.some(([status]) => status.progress?.percent === 50)).toBe(true)
    expect(updates.status()).toMatchObject({
      phase: 'downloaded',
      version: release.version,
      currentVersion: capabilities.currentVersion,
    })
    expect(updater.quitAndInstall).not.toHaveBeenCalled()
    expect(updates.shouldInstallOnQuit()).toBe(true)
    updates.install(false)
    expect(updater.autoRunAppAfterInstall).toBe(false)
    expect(updater.quitAndInstall).toHaveBeenCalledExactlyOnceWith(true, false)
    // The library's own app.quit must pass through the before-quit handler without looping.
    expect(updates.shouldInstallOnQuit()).toBe(false)
    updates.install(false)
    expect(updater.quitAndInstall).toHaveBeenCalledTimes(1)
  })

  it('honours opting out and opting back in to quit installation after the download finishes', async () => {
    const { updates, updater, preferences } = fixture({ settings: { updateInstallOnQuit: false } })
    await updates.check()
    await updates.download()
    expect(updates.shouldInstallOnQuit()).toBe(false)
    preferences({ updateInstallOnQuit: true })
    expect(updates.shouldInstallOnQuit()).toBe(true)
    preferences({ updateInstallOnQuit: false })
    expect(updates.shouldInstallOnQuit()).toBe(false)
    updates.install()
    expect(updater.autoRunAppAfterInstall).toBe(true)
    expect(updater.quitAndInstall).toHaveBeenCalledExactlyOnceWith(false, true)
  })

  it('shares concurrent checks/downloads and preserves a ready installer on later checks', async () => {
    const { updates, updater } = fixture({ settings: { updateAutoDownload: false } })
    const check = deferred<{ updateInfo: typeof release }>()
    updater.checkForUpdates.mockImplementationOnce(() => check.promise)
    const first = updates.check()
    expect(updates.check()).toBe(first)
    updater.emit('update-available', release)
    check.resolve({ updateInfo: release })
    await first
    const download = deferred<string[]>()
    updater.downloadUpdate.mockImplementationOnce(() => download.promise)
    const pending = updates.download()
    expect(updates.download()).toBe(pending)
    await updates.check()
    updater.emit('update-downloaded', release)
    download.resolve(['/verified-installer'])
    await pending
    await updates.check()
    expect(updater.checkForUpdates).toHaveBeenCalledTimes(1)
    expect(updater.downloadUpdate).toHaveBeenCalledTimes(1)
    expect(updates.status().phase).toBe('downloaded')
  })

  it('uses the download preference changed while a check is in flight', async () => {
    const { updates, updater, preferences } = fixture()
    const pending = deferred<{ updateInfo: typeof release }>()
    updater.checkForUpdates.mockImplementationOnce(() => pending.promise)
    const check = updates.check()
    preferences({ updateAutoDownload: false })
    updater.emit('update-available', release)
    pending.resolve({ updateInfo: release })
    await check
    expect(updater.downloadUpdate).not.toHaveBeenCalled()
    expect(updates.status().phase).toBe('available')
    await updates.download()
    expect(updates.status().phase).toBe('downloaded')
  })

  it('never installs an incomplete or failed verification and allows a retry', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { updates, updater } = fixture({ settings: { updateAutoDownload: false } })
    expect(() => updates.install()).toThrow('Download an update')
    await updates.check()
    updater.downloadUpdate.mockRejectedValueOnce(new Error('SHA512 checksum mismatch'))
    await updates.download()
    expect(updates.status().phase).toBe('error')
    expect(updates.shouldInstallOnQuit()).toBe(false)
    expect(() => updates.install()).toThrow('Download an update')
    expect(updater.quitAndInstall).not.toHaveBeenCalled()
    await updates.download()
    expect(updates.status().phase).toBe('downloaded')
  })

  it('handles emitted errors and offline checks without losing the last successful check time', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { updates, updater } = fixture({ settings: { updateAutoDownload: false } })
    await updates.check()
    const checkedAt = updates.status().checkedAt
    updater.checkForUpdates.mockRejectedValueOnce(new Error('Offline'))
    await updates.check()
    expect(updates.status()).toMatchObject({ phase: 'error', checkedAt })
    updater.emit('error', new Error('HTTP 429'))
    expect(updates.status().phase).toBe('error')
    await updates.check()
    expect(updates.status().phase).toBe('available')
  })

  it('keeps a verified installer ready after late errors and scheduled preference-read failures', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { updates, updater, readPreferences, onInstallFailure } = fixture()
    await updates.check()
    await updates.download()
    const ready = updates.status()
    updater.emit('error', new Error('Unrelated late network error'))
    expect(updates.status()).toEqual(ready)
    expect(updates.shouldInstallOnQuit()).toBe(true)
    readPreferences.mockRejectedValueOnce(new Error('Temporary settings read failure'))
    updates.start()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(updates.status()).toEqual(ready)
    expect(onInstallFailure).not.toHaveBeenCalled()
    updates.install()
    expect(updater.quitAndInstall).toHaveBeenCalledExactlyOnceWith(false, true)
    updates.stop()
  })

  it.each(['throw', 'emit'])(
    'reports a real installation failure through %s instead of hiding it',
    async (kind) => {
      vi.spyOn(console, 'warn').mockImplementation(() => {})
      const { updates, updater, onInstallFailure } = fixture()
      await updates.check()
      await updates.download()
      updater.quitAndInstall.mockImplementationOnce(() => {
        const failure = new Error('Installer failed')
        if (kind === 'throw') throw failure
        updater.emit('error', failure)
      })
      if (kind === 'throw') expect(() => updates.install(false)).toThrow('Installer failed')
      else updates.install(false)
      expect(updates.status().phase).toBe('error')
      expect(updates.shouldInstallOnQuit()).toBe(false)
      expect(onInstallFailure).toHaveBeenCalledTimes(1)
      await updates.download()
      expect(updates.status().phase).toBe('downloaded')
      updates.install(false)
      expect(updater.quitAndInstall).toHaveBeenLastCalledWith(true, false)
    },
  )

  it('checks on unsigned Macs but cannot download or install; development never checks', async () => {
    const mac = fixture({ canInstall: false })
    await mac.updates.check()
    await mac.updates.download()
    expect(mac.updates.status().phase).toBe('available')
    expect(mac.updater.downloadUpdate).not.toHaveBeenCalled()
    expect(mac.updates.shouldInstallOnQuit()).toBe(false)
    expect(() => mac.updates.install()).toThrow()
    const dev = fixture({ canCheck: false, canInstall: false })
    dev.updates.start()
    await dev.updates.check()
    expect(dev.updater.checkForUpdates).not.toHaveBeenCalled()
    expect(dev.updates.status().phase).toBe('disabled')
  })

  it('respects automatic-check opt-out, still allows manual checks, and stops its scheduler', async () => {
    vi.useFakeTimers()
    const { updates, updater, preferences } = fixture({
      settings: { updateAutoCheck: false, updateAutoDownload: false },
    })
    updates.start()
    updates.start()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(updater.checkForUpdates).not.toHaveBeenCalled()
    await updates.check()
    expect(updater.checkForUpdates).toHaveBeenCalledTimes(1)
    preferences({ updateAutoCheck: true })
    await vi.advanceTimersByTimeAsync(6 * 60 * 60 * 1000)
    expect(updater.checkForUpdates).toHaveBeenCalledTimes(2)
    updates.stop()
    await vi.advanceTimersByTimeAsync(6 * 60 * 60 * 1000)
    expect(updater.checkForUpdates).toHaveBeenCalledTimes(2)
  })

  it('reports up-to-date without downloading and returns defensive copies', async () => {
    const { updates, updater } = fixture()
    updater.checkForUpdates.mockImplementationOnce(async () => {
      updater.emit('update-not-available', { version: capabilities.currentVersion })
      return { updateInfo: release }
    })
    const status = await updates.check()
    expect(status.phase).toBe('up-to-date')
    status.phase = 'downloaded'
    expect(updates.status().phase).toBe('up-to-date')
    expect(updater.downloadUpdate).not.toHaveBeenCalled()
  })

  it('logs update failures with current, available, and phase context', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { updates, updater } = fixture({ settings: { updateAutoDownload: false } })
    await updates.check()
    updater.downloadUpdate.mockRejectedValueOnce(new Error('SHA512 checksum mismatch'))
    await updates.download()
    expect(updates.status().phase).toBe('error')
    expect(warn).toHaveBeenCalled()
    const line = warn.mock.calls.map((call) => String(call[0])).join('\n')
    expect(line).toContain('current=2026.10.0')
    expect(line).toContain('available=2026.10.1')
    expect(line).toContain('phase=')
  })
})

it('keeps a newer status event when it overtakes the renderer initial status request', async () => {
  const pending = deferred<AppUpdateStatus>()
  let update: (status: AppUpdateStatus) => void = () => {}
  const off = vi.fn()
  desktop({
    appUpdateStatus: () => pending.promise,
    onAppUpdate: (fn) => {
      update = fn
      return off
    },
  })
  const store = useAppUpdatesStore()
  const init = store.init()
  update({ ...capabilities, phase: 'downloaded', version: release.version })
  pending.resolve({ ...capabilities, phase: 'idle' })
  await init
  expect(store.status?.phase).toBe('downloaded')
  expect(store.attention).toBe(true)
  store.dispose()
  expect(off).toHaveBeenCalledOnce()
})
