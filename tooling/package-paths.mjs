import { posix, win32 } from 'node:path'

export function isPackageExternal(id) {
  return !id.startsWith('.') && !posix.isAbsolute(id) && !win32.isAbsolute(id)
}

export function isPackageSource(file) {
  return (
    file.endsWith('.ts') &&
    !file.endsWith('.d.ts') &&
    !/(^|[\\/])(dist|node_modules)[\\/]/.test(file)
  )
}
