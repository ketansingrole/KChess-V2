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

it('installs the Rust coverage prerequisites before every workflow check run', () => {
  for (const file of ['.github/workflows/ci.yml', '.github/workflows/release.yml']) {
    const workflow = parse(readFileSync(join(root, file), 'utf8'))
    for (const job of Object.values(workflow.jobs)) {
      const checkIndex = (job.steps ?? []).findIndex((step) => step.run === 'pnpm run check')
      if (checkIndex === -1) continue
      const before = job.steps.slice(0, checkIndex)
      expect(
        before.some((step) =>
          String(step.uses ?? '').startsWith('taiki-e/install-action@cargo-llvm-cov'),
        ),
        `${file} must install cargo-llvm-cov before pnpm run check`,
      ).toBe(true)
      expect(
        before.some((step) => String(step.run ?? '').includes('llvm-tools-preview')),
        `${file} must add llvm-tools-preview before pnpm run check`,
      ).toBe(true)
    }
  }
})

it('installs dependencies before every release job that runs a command', () => {
  const workflow = parse(readFileSync(join(root, '.github/workflows/release.yml'), 'utf8'))
  for (const [name, job] of Object.entries(workflow.jobs)) {
    if (!(job.steps ?? []).some((step) => typeof step.run === 'string')) continue
    expect(
      job.steps.some((step) => String(step.run ?? '').startsWith('pnpm install')),
      `release job ${name} must install dependencies before running commands`,
    ).toBe(true)
  }
})

it('restores the Rust build cache before every workflow step that compiles Rust', () => {
  const compiles = /^(?:pnpm run (?:check|build|build:native)|xvfb-run .*pnpm run build)$/
  for (const file of ['.github/workflows/ci.yml', '.github/workflows/release.yml']) {
    const workflow = parse(readFileSync(join(root, file), 'utf8'))
    for (const [name, job] of Object.entries(workflow.jobs)) {
      const steps = job.steps ?? []
      const first = steps.findIndex((step) => compiles.test(String(step.run ?? '')))
      if (first === -1) continue
      const cache = steps
        .slice(0, first)
        .find((step) => String(step.uses ?? '').startsWith('Swatinem/rust-cache@'))
      expect(cache, `${file} job ${name} must restore the Rust cache first`).toBeDefined()
      // Saved from main only: the repository's cache quota is shared by every branch and tag.
      expect(cache.with?.['save-if']).toBe("${{ github.ref == 'refs/heads/main' }}")
    }
  }
})

it('creates the draft release only after the full check passes', () => {
  const workflow = parse(readFileSync(join(root, '.github/workflows/release.yml'), 'utf8'))
  // Packaging runs alongside validation; the release job must still wait for both.
  expect(workflow.jobs.release.needs).toEqual(expect.arrayContaining(['validate', 'package']))
  expect(workflow.jobs.validate.steps.some((step) => step.run === 'pnpm run check')).toBe(true)
})
