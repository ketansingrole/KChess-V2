import { createHash } from 'node:crypto'
import { mkdir, readFile, writeFile, rename, rm, stat } from 'node:fs/promises'
import { join, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { unzipSync } from 'fflate'
import { create } from 'tar'

const root = fileURLToPath(new URL('..', import.meta.url))
const name = 'vosk-model-small-en-us-0.15'
const digest = '30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498'
const cache = join(root, '.data', 'voice')
const archive = join(cache, `${name}.zip`)
const output = join(root, 'public', 'voice', 'model.tar.gz')

/** Prepare once for dev/build; the packaged app never downloads a model. */
export async function prepareVoiceModel() {
  const marker = `${output}.sha256`
  try {
    if ((await readFile(marker, 'utf8')) === digest && (await stat(output)).size > 0) return
  } catch {
    // First build, or the generated asset was removed.
  }
  await mkdir(cache, { recursive: true })
  let zip
  try {
    zip = await readFile(archive)
  } catch {
    console.info('[kchess] Downloading the offline English voice model (40 MB)…')
    const response = await fetch(`https://alphacephei.com/vosk/models/${name}.zip`, {
      signal: AbortSignal.timeout(300_000),
    })
    if (!response.ok) throw new Error(`Voice model download failed: HTTP ${response.status}`)
    zip = Buffer.from(await response.arrayBuffer())
  }
  if (createHash('sha256').update(zip).digest('hex') !== digest)
    throw new Error(`Voice model checksum mismatch. Remove ${archive} and try again.`)
  await writeFile(archive, zip)
  const extracted = join(cache, 'unpacked')
  await rm(extracted, { recursive: true, force: true })
  await mkdir(extracted, { recursive: true })
  try {
    for (const [path, data] of Object.entries(unzipSync(zip))) {
      if (path.endsWith('/')) continue
      const target = resolve(extracted, path)
      if (!target.startsWith(`${resolve(extracted)}${sep}`)) throw new Error('Invalid model path')
      await mkdir(resolve(target, '..'), { recursive: true })
      await writeFile(target, data)
    }
    await mkdir(resolve(output, '..'), { recursive: true })
    await create({ cwd: extracted, file: `${output}.tmp`, gzip: true, portable: true }, [name])
    await rename(`${output}.tmp`, output)
    await writeFile(marker, digest)
    console.info('[kchess] Offline voice model ready.')
  } finally {
    await rm(extracted, { recursive: true, force: true })
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  await prepareVoiceModel()
