import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { parseReleaseVersion } from './release-rules.mjs'

export function verifyReleaseSnap(tag, checksums, bytes) {
  if (!tag?.startsWith('v')) throw new Error('Missing release tag.')
  parseReleaseVersion(tag.slice(1))
  const name = `KChess-${tag.slice(1)}-linux-x64.snap`
  const entries = checksums.split('\n').filter((line) => line.endsWith(`  ${name}`))
  const digest = createHash('sha256').update(bytes).digest('hex')
  if (entries.length !== 1 || entries[0] !== `${digest}  ${name}`)
    throw new Error('Snap checksum is missing, ambiguous or incorrect.')
  return name
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const tag = process.env.RELEASE_TAG
  if (!tag?.startsWith('v')) throw new Error('Missing release tag.')
  parseReleaseVersion(tag.slice(1))
  const name = `KChess-${tag.slice(1)}-linux-x64.snap`
  const latest = JSON.parse(
    execFileSync('gh', ['api', `repos/${process.env.GH_REPO}/releases/latest`], {
      encoding: 'utf8',
    }),
  )
  if (latest.tag_name !== tag)
    throw new Error('Refusing to publish an older release over latest/stable.')
  mkdirSync('packages', { recursive: true })
  execFileSync(
    'gh',
    [
      'release',
      'download',
      tag,
      '--pattern',
      name,
      '--pattern',
      'SHA256SUMS.txt',
      '--dir',
      'packages',
    ],
    { stdio: 'inherit' },
  )
  verifyReleaseSnap(
    tag,
    readFileSync('packages/SHA256SUMS.txt', 'utf8'),
    readFileSync(`packages/${name}`),
  )
}
