import { existsSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { parse } from 'yaml'
import { expect, it } from 'vitest'
import { validateReleaseVersion } from '../../tooling/release-rules.mjs'

const root = process.cwd()
const readPackage = (directory) =>
  JSON.parse(readFileSync(join(root, directory, 'package.json'), 'utf8'))

it('verifies every shipped package against its own lockfile importer and release version', () => {
  const lock = parse(readFileSync(join(root, 'pnpm-lock.yaml'), 'utf8'))
  const desktop = readPackage('apps/desktop')
  const version = readPackage('.').version
  for (const directory of [
    '.',
    'crates/kchess-contracts/ts',
    'crates/kchess-wasm/js',
    'hosts/node',
    'apps/cli',
    'apps/desktop',
    'crates/kchess-node',
  ]) {
    const pkg = readPackage(directory)
    expect(pkg.version).toBe(version)
    expect(validateReleaseVersion(pkg, lock, `v${version}`, directory, desktop)).toBe(`v${version}`)
  }
})

it('resolves the release configuration from the desktop project directory', () => {
  const workflow = parse(readFileSync(join(root, '.github/workflows/release.yml'), 'utf8'))
  const command = workflow.jobs.package.steps.find(
    (step) => step.name === 'Build installers and verify package contents',
  ).run
  const project = /--projectDir\s+(\S+)/.exec(command)?.[1]
  const config = /--config\s+(\S+)/.exec(command)?.[1]
  expect(project).toBe('apps/desktop')
  expect(config).toBeDefined()
  expect(existsSync(resolve(root, project, config))).toBe(true)
})
