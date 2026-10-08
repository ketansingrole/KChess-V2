import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { beforeEach, expect, it, vi } from 'vitest'
import { dialog } from 'electron'
import { saveWithDialog } from '../../src/main/saveDialog'

vi.mock('electron', () => ({ dialog: { showSaveDialog: vi.fn() } }))

const request = { title: 'Export', defaultPath: 'a.json', filters: [], data: '{}\n' }

beforeEach(() => vi.mocked(dialog.showSaveDialog).mockReset())

it('reports a cancelled save as false', async () => {
  vi.mocked(dialog.showSaveDialog).mockResolvedValueOnce({ canceled: true } as never)
  expect(await saveWithDialog(null, request)).toBe(false)
})

it('writes the document to the chosen path', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'kchess-save-'))
  try {
    const filePath = join(directory, 'out.json')
    vi.mocked(dialog.showSaveDialog).mockResolvedValueOnce({ canceled: false, filePath } as never)
    expect(await saveWithDialog(null, request)).toBe(true)
    expect(await readFile(filePath, 'utf8')).toBe('{}\n')
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
