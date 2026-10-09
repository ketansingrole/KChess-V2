import type { CoreSettings, CoreData, AppData } from './types'
export const CORE_SETTINGS_KEYS = [
  'enginePath',
  'engineLevels',
  'reviewAuto',
  'reviewOnBattery',
  'receiveChallenges',
  'onlineChat',
  'correspondencePoll',
  'cloudEval',
  'voiceHistory',
] as const satisfies readonly (keyof CoreSettings)[]
export function coreSettings(settings: CoreSettings): CoreSettings {
  return Object.fromEntries(
    CORE_SETTINGS_KEYS.map((key) => [
      key,
      key === 'engineLevels' ? [...settings.engineLevels] : settings[key],
    ]),
  ) as unknown as CoreSettings
}
export function coreData(data: AppData | CoreData): CoreData {
  return { ...data, settings: coreSettings(data.settings) }
}
