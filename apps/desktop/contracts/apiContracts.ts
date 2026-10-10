import * as v from 'valibot'
import { CORE_CONTRACTS, validateArguments, type Checks } from '@kchess/contracts/apiContracts'
import { assertExport, assertNotification, assertSettings } from '@kchess/rules/validate'
import { PERFORMANCE_NAMES, RENDERER_ROUTES } from './rendererDiagnostics'
import type { InvokeMethod, IpcArguments } from './ipc'

type Contracts = { [K in InvokeMethod]: { min: number; checks: Checks<IpcArguments<K>> } }
const boolean = (value: unknown) => v.parse(v.boolean(), value)
const timingName = (value: unknown) => v.parse(v.picklist(PERFORMANCE_NAMES), value)
const timingValue = (value: unknown) =>
  v.parse(v.pipe(v.number(), v.finite(), v.minValue(0), v.maxValue(300_000)), value)

export const IPC_CONTRACTS = {
  ...CORE_CONTRACTS,
  completeQuit: {
    min: 1,
    checks: [(value: unknown) => v.parse(v.pipe(v.string(), v.uuid()), value)],
  },
  recordPerformance: { min: 2, checks: [timingName, timingValue] },
  reportRendererError: {
    min: 1,
    checks: [
      (value) =>
        v.parse(
          v.strictObject({
            route: v.picklist(RENDERER_ROUTES),
            message: v.pipe(v.string(), v.maxLength(1000)),
            info: v.pipe(v.string(), v.maxLength(160)),
          }),
          value,
        ),
    ],
  },
  saveExport: { min: 1, checks: [assertExport] },
  exportDiagnostics: { min: 0, checks: [] },
  windowMinimize: { min: 0, checks: [] },
  windowToggleMaximize: { min: 0, checks: [] },
  windowClose: { min: 0, checks: [] },
  windowIsMaximized: { min: 0, checks: [] },
  appUpdateStatus: { min: 0, checks: [] },
  checkAppUpdate: { min: 0, checks: [] },
  downloadAppUpdate: { min: 0, checks: [] },
  installAppUpdate: { min: 0, checks: [] },
  openAppReleases: { min: 0, checks: [] },
  chooseEngine: { min: 0, checks: [] },
  notify: { min: 1, checks: [assertNotification] },
  loadThemes: { min: 0, checks: [] },
  openThemesFolder: { min: 0, checks: [] },
  openNotificationSettings: { min: 0, checks: [] },
  microphoneAccess: { min: 1, checks: [boolean] },
  ensureVoiceModel: { min: 0, checks: [] },
  voiceModelStatus: { min: 0, checks: [] },
  openMicrophoneSettings: { min: 0, checks: [] },
  exportVoiceHistory: { min: 0, checks: [] },
  saveSettings: { min: 1, checks: [assertSettings] },
} satisfies Contracts

export function validateIpcArguments(method: InvokeMethod, args: unknown[]): void {
  validateArguments(IPC_CONTRACTS, method, args)
}
