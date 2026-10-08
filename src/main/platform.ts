import { app, powerMonitor, safeStorage, shell } from 'electron'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { BUNDLED_ENGINE_SCRIPT, type CorePlatform } from '../core'

/** The core's host capabilities, backed by Electron. */
export function electronPlatform(focus: () => void): CorePlatform {
  return {
    dataDir: app.getPath('userData'),
    // Packaged apps keep the engine outside the asar so a child process can load its .wasm.
    bundledEnginePath: join(app.getAppPath(), BUNDLED_ENGINE_SCRIPT).replace(
      /app\.asar([\\/])/,
      'app.asar.unpacked$1',
    ),
    // Earlier releases installed the downloaded engine here; keep using it.
    managedEngineDir: join(homedir(), '.kchess/engines/stockfish/current'),
    // The Electron binary runs the engine script as plain Node.
    nodeEnv: { ELECTRON_RUN_AS_NODE: '1' },
    // Earlier macOS releases kept their database here; an isolated profile never imports it.
    legacyDatabasePath:
      process.platform === 'darwin' && !process.env.KCHESS_USER_DATA_DIR
        ? join(homedir(), '.kchess', 'kchess.db')
        : undefined,
    secrets: {
      // `safeStorage` reports "available" on Linux even with its plaintext fallback backend.
      available: () =>
        safeStorage.isEncryptionAvailable() &&
        !(process.platform === 'linux' && safeStorage.getSelectedStorageBackend() === 'basic_text'),
      encrypt: (plain) => safeStorage.encryptString(plain).toString('base64'),
      decrypt: (base64) => safeStorage.decryptString(Buffer.from(base64, 'base64')),
    },
    openExternal: (url) => shell.openExternal(url),
    focus,
    onBattery: () => powerMonitor.isOnBatteryPower(),
  }
}
