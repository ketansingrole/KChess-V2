export interface ReleaseAsset {
  name: string
  browser_download_url: string
  size: number
  digest?: string | null
}

/** Pick the macOS build for this CPU; the universal binary runs on both architectures. */
export function pickMacAsset(assets: ReleaseAsset[], arch: string): ReleaseAsset | undefined {
  const preferred =
    arch === 'arm64'
      ? [/macos.*m1-apple-silicon/i, /macos.*apple-silicon/i, /macos.*arm64/i]
      : [/macos.*x86-64/i, /macos.*x64/i]
  for (const pattern of [...preferred, /macos.*universal/i]) {
    const asset = assets.find((a) => pattern.test(a.name))
    if (asset) return asset
  }
  return undefined
}
