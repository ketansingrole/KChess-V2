import { parentPort, workerData } from 'node:worker_threads'
import { createHash } from 'node:crypto'
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises'
import { dirname, join, resolve, sep } from 'node:path'
import { unzipSync } from 'fflate'
import { create } from 'tar'

const MAX_EXTRACTED = 200 * 1024 * 1024
const { archive, directory, model } = workerData as {
  archive: string
  directory: string
  model: { name: string; sha256: string }
}
async function prepare(): Promise<void> {
  const zip = await readFile(archive)
  if (createHash('sha256').update(zip).digest('hex') !== model.sha256)
    throw new Error('Voice model checksum mismatch. Try downloading it again.')
  const extracted = join(directory, 'unpacked')
  let bytes = 0
  const files = unzipSync(zip, {
    filter: (entry) => {
      bytes += entry.originalSize
      const path = resolve(extracted, entry.name)
      if (
        bytes > MAX_EXTRACTED ||
        !entry.name.startsWith(`${model.name}/`) ||
        !path.startsWith(`${resolve(extracted, model.name)}${sep}`) ||
        entry.name.includes('\\')
      ) {
        // ZIPs contain a directory entry for the model's root.
        if (entry.name === `${model.name}/` && !entry.originalSize) return false
        throw new Error('Invalid voice model archive.')
      }
      return !entry.name.endsWith('/')
    },
  })
  if (!Object.keys(files).length) throw new Error('Voice model archive is empty.')
  for (const [path, data] of Object.entries(files)) {
    const target = resolve(extracted, path)
    await mkdir(dirname(target), { recursive: true })
    await writeFile(target, data)
  }
  const prepared = join(directory, 'model.tar.gz')
  await create({ cwd: extracted, file: prepared, gzip: true, portable: true }, [model.name])
  const marker = {
    source: model.sha256,
    sha256: createHash('sha256')
      .update(await readFile(prepared))
      .digest('hex'),
    size: (await stat(prepared)).size,
  }
  parentPort!.postMessage({ marker, prepared })
}
void prepare()
  .catch((cause: unknown) => {
    parentPort!.postMessage({ error: cause instanceof Error ? cause.message : String(cause) })
  })
  .finally(() => parentPort!.close())
