import { app } from 'electron'
import { mkdir, readdir, readFile, stat, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { logDebug, logWarn } from './logger'
import { assertTheme } from '../shared/validate'
import type { AppTheme, CustomThemeReport } from '../shared/types'

const MAX_FILES = 100
const MAX_BYTES = 64 * 1024

/** Written once so there is something to copy. Files starting with `_` are never loaded. */
const TEMPLATE = {
  id: 'my-theme',
  name: 'My theme',
  dark: {
    bg: '#1b1d23',
    text: '#e6e8ee',
    primary: '#7aa2f7',
    muted: '#20232b',
    elevated: '#262a33',
    accented: '#323744',
    border: '#343946',
    textMuted: '#9aa1b2',
    success: '#7fd28a',
    warning: '#e6b35a',
    error: '#ef7b86',
    info: '#6cb6ff',
  },
  light: {
    bg: '#fbfbfd',
    text: '#22252e',
    primary: '#3b5fd0',
  },
}

export function themesDir(): string {
  return join(app.getPath('userData'), 'themes')
}

/** Read every valid `*.json` theme in the themes folder; a bad file is reported, never fatal. */
export async function loadCustomThemes(): Promise<CustomThemeReport> {
  const dir = themesDir()
  const themes: AppTheme[] = []
  const problems: string[] = []
  await mkdir(dir, { recursive: true })
  const template = join(dir, '_template.json')
  await stat(template).catch((cause: unknown) => {
    logDebug('themes', 'Theme template is missing, creating it:', cause)
    return writeFile(template, `${JSON.stringify(TEMPLATE, null, 2)}\n`, 'utf8')
  })
  const files = (await readdir(dir))
    .filter((name) => name.endsWith('.json') && !name.startsWith('_') && !name.startsWith('.'))
    .sort()
  if (files.length > MAX_FILES) problems.push(`Only the first ${MAX_FILES} theme files are read.`)
  for (const name of files.slice(0, MAX_FILES)) {
    try {
      const path = join(dir, name)
      if ((await stat(path)).size > MAX_BYTES) throw new Error('The file is too large.')
      const theme = assertTheme(JSON.parse(await readFile(path, 'utf8')))
      themes.push({ ...theme, custom: true })
    } catch (cause) {
      logWarn('themes', 'Skipping invalid theme file:', name, cause)
      problems.push(`${name}: ${cause instanceof Error ? cause.message : 'unreadable'}`)
    }
  }
  return { themes, dir, problems }
}
