import { dialog, type BrowserWindow, type FileFilter } from 'electron'
import { writeFile } from 'node:fs/promises'

export interface SaveRequest {
  title: string
  defaultPath: string
  filters: FileFilter[]
  data: string | Uint8Array
}

/** Write a document where the native save dialog says; false when cancelled. */
export async function saveWithDialog(
  window: BrowserWindow | null,
  { data, ...options }: SaveRequest,
): Promise<boolean> {
  const result =
    window && !window.isDestroyed()
      ? await dialog.showSaveDialog(window, options)
      : await dialog.showSaveDialog(options)
  if (result.canceled || !result.filePath) return false
  await writeFile(result.filePath, data)
  return true
}
