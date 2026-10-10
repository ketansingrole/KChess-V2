import type { VoiceModelProgress, VoiceModelStatus } from '@kchess/contracts/types'
import { logDebug, logError, logInfo, logWarn } from './logger.ts'
import { nativeRules, type NativeRules } from './native.ts'

/**
 * The offline voice model for voice input. The cache is Rust (`crates/kchess-core/src/voice`):
 * the verified download, unpacking, the `model.tar.gz` and its marker, status and progress.
 * This module keeps the TypeScript names the desktop shell and the tooling use; it has no
 * fallback, as the core requires `@kchess/native` (`RUST_MIGRATION.md`).
 */

export interface VoiceModelInfo {
  name: string
  url: string
  sha256: string
}

const LOG = { debug: logDebug, info: logInfo, warn: logWarn, error: logError } as const

function rules(): NativeRules {
  const native = nativeRules()
  if (!native) throw new Error('The native rules are not built (pnpm run build:native).')
  return native
}

/** The model KChess downloads (`VoiceModel::official` in Rust). */
export const VOICE_MODEL: VoiceModelInfo = JSON.parse(
  rules().voiceModelMetadata(),
) as VoiceModelInfo

export interface VoiceModelCacheOptions {
  /** A local zip used instead of the download (the e2e fixture from `tooling/prepare-voice-model.mjs`). */
  archivePath?: string
}

/** One verified, persistent model per profile. Concurrent voice controls share preparation. */
export class VoiceModelCache {
  /** The published archive, served to the renderer as `kchess://app/voice/model.tar.gz`. */
  readonly path: string
  private readonly model: InstanceType<NativeRules['NativeVoiceModel']>

  constructor(directory: string, options: VoiceModelCacheOptions = {}) {
    this.model = new (rules().NativeVoiceModel)(directory, options, (json) => {
      const line = JSON.parse(json) as { level: keyof typeof LOG; scope: string; message: string }
      LOG[line.level](line.scope, line.message)
    })
    this.path = this.model.path
  }

  /** Verify, or download and prepare, the model; resolves with its path. */
  ensure(listener: (progress: VoiceModelProgress) => void = () => {}): Promise<string> {
    return this.model.ensure((json) => listener(JSON.parse(json) as VoiceModelProgress))
  }

  /** Whether a verified model is installed, without downloading anything. */
  async status(): Promise<VoiceModelStatus> {
    return JSON.parse(await this.model.status()) as VoiceModelStatus
  }
}
