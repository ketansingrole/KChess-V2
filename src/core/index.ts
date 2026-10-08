export { createKChessCore, type KChessCore } from './service'
export type { CorePlatform, SecretStore } from './platform'
export { BUNDLED_ENGINE_SCRIPT } from './engine'
export { CORE_METHODS, type CoreEvents, type CoreMethod } from '../shared/core'
export type { CoreApi } from '../shared/types'
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
