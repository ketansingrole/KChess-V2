import type { CoreSettings, CoreData, AppData } from './types'
import { CORE_SETTINGS_KEYS } from './generated/settings.ts'

export * from './generated/settings.ts'

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
