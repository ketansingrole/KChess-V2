#!/usr/bin/env node
// Writes the Lucide icons the GPUI app names (`icon("name")` in apps/gpui/src) as SVG files in
// apps/gpui/assets/icons, from the same @iconify-json/lucide set the Electron renderer draws its
// `i-lucide-*` icons with, so both apps show identical glyphs. Aliases resolve as Iconify
// resolves them (`history` is `rotate-ccw-clock`). `--check` fails when the files are stale.
import { readFileSync, readdirSync, writeFileSync, mkdirSync, rmSync, existsSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(join(root, 'package.json'))
const set = require('@iconify-json/lucide/icons.json')
const check = process.argv.includes('--check')
const out = join(root, 'apps/gpui/assets/icons')

function sources(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) return sources(path)
    return entry.name.endsWith('.rs') ? [path] : []
  })
}

const names = new Set()
for (const file of sources(join(root, 'apps/gpui/src')))
  for (const match of readFileSync(file, 'utf8').matchAll(/\bicon\("([a-z0-9-]+)"\)/g))
    names.add(match[1])

function resolve(name, seen = new Set()) {
  if (set.icons[name]) return set.icons[name]
  const alias = set.aliases?.[name]
  if (!alias || seen.has(name)) throw new Error(`Lucide has no icon named ${name}.`)
  seen.add(name)
  return resolve(alias.parent, seen)
}

const width = set.width ?? 24
const height = set.height ?? 24
const wanted = new Map(
  [...names].sort().map((name) => {
    const icon = resolve(name)
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${icon.width ?? width}" height="${icon.height ?? height}" viewBox="0 0 ${icon.width ?? width} ${icon.height ?? height}">${icon.body}</svg>\n`
    return [`${name}.svg`, svg]
  }),
)

const present = existsSync(out) ? readdirSync(out).filter((name) => name.endsWith('.svg')) : []
const stale = [
  ...[...wanted].filter(([file, svg]) => {
    const path = join(out, file)
    return !existsSync(path) || readFileSync(path, 'utf8') !== svg
  }),
].map(([file]) => file)
const unused = present.filter((file) => !wanted.has(file))

if (check) {
  if (stale.length || unused.length) {
    console.error(
      `GPUI icons are out of date (${[...stale, ...unused].join(', ')}). Run node tooling/gpui-icons.mjs.`,
    )
    process.exit(1)
  }
  console.log(`GPUI icons: ${wanted.size} up to date.`)
} else {
  mkdirSync(out, { recursive: true })
  for (const file of unused) rmSync(join(out, file))
  for (const [file, svg] of wanted) writeFileSync(join(out, file), svg)
  console.log(`GPUI icons: wrote ${wanted.size}, removed ${unused.length}.`)
}
