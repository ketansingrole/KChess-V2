// The development badge's markers, kept free of dependencies so GPUI packaging (which runs
// without `pnpm install`) can check its release binary too.
import { readFile } from 'node:fs/promises'

/**
 * Names only the development builds' sidebar badge carries: the Electron renderer's
 * (`DevBuildBadge.vue`) and the GPUI app's (`shell.rs`). A release that contains one shows it.
 */
const DEV_BUILD_MARKERS = ['data-dev-build', 'kchess-dev-build-badge']

/** The development marker `bytes` (a renderer file or a release binary) contains, if any. */
export function devBuildMarker(bytes) {
  const buffer = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes)
  return DEV_BUILD_MARKERS.find((marker) => buffer.includes(marker)) ?? null
}

export async function verifyNoDevBuild(path, label) {
  const marker = devBuildMarker(await readFile(path))
  if (marker) throw new Error(`${label} contains the development build badge (${marker}): ${path}`)
}
