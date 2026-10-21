import { readdirSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { join } from 'node:path'

/** Include icons in data arrays/conditional props that the template scanner cannot discover. */
export function localIcons(): string[] {
  const root = fileURLToPath(new URL('../apps/desktop/app', import.meta.url))
  const icons = new Set<string>()
  for (const path of readdirSync(root, { recursive: true, encoding: 'utf8' })) {
    if (!/\.(ts|vue)$/.test(path)) continue
    for (const match of readFileSync(join(root, path), 'utf8').matchAll(/i-lucide-([a-z0-9-]+)/g))
      icons.add(`lucide:${match[1]}`)
  }
  return [...icons].sort()
}
