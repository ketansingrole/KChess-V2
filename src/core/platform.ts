/**
 * What the headless core needs from its host. The desktop shell supplies an Electron
 * implementation; other frontends supply their own. Core modules never import a frontend.
 */
export interface CorePlatform {
  /** Directory for KChess's databases and other local data; it must exist. */
  dataDir: string
  /** The bundled Stockfish UCI script, run with this process's Node runtime. */
  bundledEnginePath: string
  /** Extra environment for running the bundled engine with `process.execPath`. */
  nodeEnv?: Record<string, string>
  /** Where KChess keeps the Stockfish build it downloads; defaults to the data directory. */
  managedEngineDir?: string
  /** The built puzzle worker (`puzzleWorker.ts`); by default, `puzzleWorker.js` beside the core. */
  puzzleWorkerPath?: string
  /** OS-backed encryption for stored Lichess tokens. */
  secrets: SecretStore
  /** Open a URL in the user's browser (Lichess sign-in). */
  openExternal(url: string): Promise<void>
  /** A database from an earlier KChess release, imported once into an empty profile. */
  legacyDatabasePath?: string
  /** Bring the frontend back to the foreground after the browser sign-in returns. */
  focus?(): void
  /** Background engine work yields while on battery power. */
  onBattery(): boolean
}

export interface SecretStore {
  /** False when only a plaintext fallback exists; tokens are then never stored. */
  available(): boolean
  /** Base64 ciphertext. */
  encrypt(plain: string): string
  decrypt(base64: string): string
}

let current: CorePlatform | undefined

export function setPlatform(platform: CorePlatform): void {
  current = platform
}

export function platform(): CorePlatform {
  if (!current) throw new Error('The KChess core has not been configured with a platform.')
  return current
}
