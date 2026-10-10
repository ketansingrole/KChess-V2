export { createKChessCore, type KChessCore } from './service'
export type { CorePlatform, SecretStore } from './platform'
export { BUNDLED_ENGINE_SCRIPT } from './engine'
export { CORE_METHODS, type CoreEvents, type CoreMethod } from '@kchess/contracts/core'
export type { CoreApi } from '@kchess/contracts/types'
// Diagnostics infrastructure the frontends share with the core.
export {
  errorSummary,
  isExpectedCancellation,
  logDebug,
  logError,
  logInfo,
  logWarn,
  truncateForLog,
} from './logger'
export { performanceSnapshot, recordTiming, startPerformanceMonitoring } from './performance'
export { DiagnosticLog } from './diagnosticLog'
export { VOICE_MODEL, VoiceModelCache } from './voiceModel'
export { validateCoreArguments } from '@kchess/contracts/apiContracts'
export { assertLevel } from '@kchess/rules/validate'
export type { ArchivedGame } from '@kchess/rules/library'
