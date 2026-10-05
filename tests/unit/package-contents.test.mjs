import { describe, expect, it } from 'vitest'
import { verifyEntries, verifyRendererEntries } from '../../scripts/verify-package.mjs'

describe('release payload checks', () => {
  it('accepts frontend assets, runtime dependencies, and the lite engine', () => {
    expect(
      verifyEntries([
        { path: '.output/public/_nuxt/app.js', size: 5000 },
        { path: 'node_modules/electron-updater/out/main.js', size: 2000 },
        { path: 'node_modules/stockfish/bin/stockfish-19-lite.wasm', size: 1636291 },
      ]),
    ).toBeGreaterThan(0)
  })

  it.each([
    '.output/public/mac-arm64/Electron.app/Contents/MacOS/Electron',
    '.output/public/mac-arm64/KChess.app/Contents/Resources/app.asar',
    '.output/public/.temp0q2axekpKChess-2026.10.0-mac-arm64.dmg',
    '.output/public/KChess.zip',
    '.output/public/voice/model.tar.gz',
    'node_modules/vosk-browser/dist/vosk.js',
    'node_modules/@nuxt/kit/dist/index.mjs',
    'node_modules/pinia/dist/pinia.mjs',
    'node_modules/stockfish/bin/stockfish-19.wasm',
  ])('rejects unwanted payload %s', (path) => {
    expect(() => verifyEntries([{ path, size: 1 }])).toThrow()
  })

  it('fails oversized individual files and aggregate payloads', () => {
    expect(() =>
      verifyEntries([{ path: '.output/public/large.bin', size: 17 * 1024 * 1024 }]),
    ).toThrow('large file')
    expect(() =>
      verifyEntries(
        Array.from({ length: 4 }, (_, index) => ({
          path: `.output/public/chunk${index}.bin`,
          size: 10 * 1024 * 1024,
        })),
      ),
    ).toThrow('32 MiB budget')
  })
})

it('enforces the renderer budget separately from the larger packaged runtime budget', () => {
  const entries = [{ path: '_nuxt/app.js', size: 13 * 1024 * 1024 }]
  expect(() => verifyEntries(entries)).not.toThrow()
  expect(() => verifyRendererEntries(entries)).toThrow('12 MiB')
})
