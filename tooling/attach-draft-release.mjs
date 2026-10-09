import { execFileSync } from 'node:child_process'
import { readdirSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseReleaseVersion } from './release-rules.mjs'

const gh = (...args) => execFileSync('gh', args, { encoding: 'utf8' })

/** A successful listing is required before any mutation; published releases are never overwritten. */
export function attachDraftRelease(tag, files, run = gh) {
  if (!tag?.startsWith('v')) throw new Error('A release tag must start with v.')
  parseReleaseVersion(tag.slice(1))
  if (!files.length) throw new Error('No release assets were provided.')
  const releases = JSON.parse(
    run('release', 'list', '--limit', '1000', '--json', 'tagName,isDraft'),
  )
  if (!Array.isArray(releases)) throw new Error('Could not verify existing releases.')
  const existing = releases.find((release) => release.tagName === tag)
  if (existing) {
    if (existing.isDraft !== true)
      throw new Error(
        `Release ${tag} is already published. Advance the version and create a new tag; its assets will not be overwritten.`,
      )
    run('release', 'upload', tag, ...files, '--clobber')
  } else {
    // --verify-tag prevents gh from inventing a tag at the default branch's HEAD.
    run(
      'release',
      'create',
      tag,
      ...files,
      '--verify-tag',
      '--draft',
      '--generate-notes',
      '--title',
      `KChess ${tag.slice(1)}`,
    )
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const directory = process.argv[2] || 'packages'
  const tag = process.env.GITHUB_REF_NAME
  if (process.env.GITHUB_EVENT_NAME !== 'push' || process.env.GITHUB_REF !== `refs/tags/${tag}`)
    throw new Error(
      'Draft releases may only be attached by a tag-push workflow, never a manual build.',
    )
  attachDraftRelease(
    tag,
    readdirSync(directory).map((file) => join(directory, file)),
  )
}
