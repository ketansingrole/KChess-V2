import { describe, expect, it, vi } from 'vitest'
import {
  nextReleaseVersion,
  parseReleaseVersion,
  validateReleaseVersion,
} from '../../scripts/release-rules.mjs'
import { attachDraftRelease } from '../../scripts/attach-draft-release.mjs'
import { prepareNextVersion } from '../../scripts/next-version.mjs'

const pkg = {
  version: '2026.10.1',
  build: {
    mac: {
      target: [
        { target: 'dmg', arch: ['arm64'] },
        { target: 'zip', arch: ['arm64'] },
      ],
    },
  },
}
const lock = { version: pkg.version, packages: { '': { version: pkg.version } } }

describe('release rules', () => {
  it('leaves package versions untouched when refreshing tags fails', () => {
    const git = vi.fn(() => {
      throw new Error('cannot reach origin')
    })
    const write = vi.fn()
    expect(() => prepareNextVersion(git, write)).toThrow('Version was not changed')
    expect(write).not.toHaveBeenCalled()
    expect(git).toHaveBeenCalledOnce()
  })

  it('writes the next version only after successfully fetching origin tags', () => {
    const git = vi.fn().mockReturnValueOnce('').mockReturnValueOnce('v2026.10.0\nv2026.10.2')
    const write = vi.fn()
    expect(prepareNextVersion(git, write, new Date('2026-10-02T00:00:00Z'))).toBe('2026.10.3')
    expect(git.mock.calls[0]).toEqual(['fetch', 'origin', '--tags', '--quiet'])
    expect(write).toHaveBeenCalledExactlyOnceWith('2026.10.3')
  })

  it.each(['2026.10.0', '2026.1.12', '2026.12.3'])('accepts %s', (version) => {
    expect(parseReleaseVersion(version).year).toBe(2026)
  })
  it.each([
    'v2026.10.1',
    '1.2.3',
    '2026.01.0',
    '2026.10.01',
    '2026.0.0',
    '2026.13.0',
    '2026.10.1-beta',
    '2026.10.1.1',
    '2026.10.1\n',
    '2026.10.-1',
    '2026.10.9007199254740992',
  ])('rejects %s', (version) => {
    expect(() => parseReleaseVersion(version)).toThrow('Invalid release version')
  })
  it('uses the highest counter, ignores non-release tags, and resets each UTC month', () => {
    const tags = ['v2026.10.0', 'v2026.10.2', 'v2026.10.1', 'v2026.10.03', 'v1.2.3']
    expect(nextReleaseVersion(tags, new Date('2026-10-02T00:00:00Z'))).toBe('2026.10.3')
    expect(nextReleaseVersion(tags, new Date('2026-11-01T00:00:00Z'))).toBe('2026.11.0')
    expect(nextReleaseVersion(['v2026.9.2'], new Date('2026-10-01T00:30:00+09:00'))).toBe(
      '2026.9.3',
    )
  })
  it('refuses to version backwards across a future tag', () => {
    expect(() => nextReleaseVersion(['v2026.11.0'], new Date('2026-10-02T00:00:00Z'))).toThrow(
      'ahead',
    )
  })
  it('requires matching tag, package version, and both lockfile fields', () => {
    expect(validateReleaseVersion(pkg, lock, 'v2026.10.1')).toBe('v2026.10.1')
    expect(() => validateReleaseVersion(pkg, lock, 'v2026.10.0')).toThrow('exactly')
    expect(() => validateReleaseVersion(pkg, { ...lock, version: '2026.10.0' })).toThrow(
      'must match',
    )
    expect(() =>
      validateReleaseVersion(pkg, { ...lock, packages: { '': { version: '2026.10.0' } } }),
    ).toThrow('must match')
    expect(() =>
      validateReleaseVersion(
        { ...pkg, build: { mac: { target: [{ arch: ['arm64', 'x64'] }] } } },
        lock,
      ),
    ).toThrow('Apple Silicon')
  })
})

describe('draft release uploads', () => {
  it('creates only a draft and verifies the tag already exists remotely', () => {
    const run = vi.fn().mockReturnValueOnce('[]')
    attachDraftRelease('v2026.10.1', ['packages/app.dmg'], run)
    expect(run.mock.calls[1]).toEqual(expect.arrayContaining(['create', '--draft', '--verify-tag']))
  })
  it('permits replacement only on an existing draft', () => {
    const run = vi
      .fn()
      .mockReturnValueOnce(JSON.stringify([{ tagName: 'v2026.10.1', isDraft: true }]))
    attachDraftRelease('v2026.10.1', ['packages/app.dmg'], run)
    expect(run.mock.calls[1]).toEqual([
      'release',
      'upload',
      'v2026.10.1',
      'packages/app.dmg',
      '--clobber',
    ])
  })
  it('does not mutate a published release', () => {
    const run = vi
      .fn()
      .mockReturnValueOnce(JSON.stringify([{ tagName: 'v2026.10.1', isDraft: false }]))
    expect(() => attachDraftRelease('v2026.10.1', ['packages/app.dmg'], run)).toThrow(
      'already published',
    )
    expect(run).toHaveBeenCalledOnce()
  })
  it('fails closed if inspecting releases fails or returns invalid data', () => {
    const run = vi.fn(() => {
      throw new Error('network failure')
    })
    expect(() => attachDraftRelease('v2026.10.1', ['packages/app.dmg'], run)).toThrow(
      'network failure',
    )
    expect(run).toHaveBeenCalledOnce()
    const invalid = vi.fn().mockReturnValue('null')
    expect(() => attachDraftRelease('v2026.10.1', ['packages/app.dmg'], invalid)).toThrow(
      'Could not verify',
    )
    expect(invalid).toHaveBeenCalledOnce()
  })
})
