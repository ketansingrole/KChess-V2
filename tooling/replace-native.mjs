import { copyFileSync, renameSync, rmSync } from 'node:fs'

/** Keep loaded native code intact, including macOS's cached code signature, during rebuilds. */
export function replaceNative(source, target) {
  const temporary = `${target}.${process.pid}.tmp`
  try {
    copyFileSync(source, temporary)
    renameSync(temporary, target)
  } finally {
    rmSync(temporary, { force: true })
  }
}
