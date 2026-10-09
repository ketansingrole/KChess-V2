import { readFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { VoiceModelCache, VOICE_MODEL } from '../core/src/services/voiceModel.ts'

const root = fileURLToPath(new URL('..', import.meta.url))
export const voiceFixtureDirectory = join(root, '.data', 'voice', 'cache')

/** Prepare a verified local fixture for voice e2e tests, outside all packaged assets. */
export async function prepareVoiceModel() {
  const cache = new VoiceModelCache(voiceFixtureDirectory, async (url, options) => {
    try {
      const zip = await readFile(join(root, '.data', 'voice', `${VOICE_MODEL.name}.zip`))
      return new Response(zip)
    } catch {
      return fetch(url, options)
    }
  })
  return cache.ensure()
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.info('[kchess] Preparing the voice test fixture (first download: about 40 MB)…')
  console.info(`[kchess] Voice test model ready: ${await prepareVoiceModel()}`)
}
