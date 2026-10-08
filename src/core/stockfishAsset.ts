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

/** Conservative universal builds dispatch at runtime to supported CPU instructions. */
export function pickStockfishAsset(
  assets: ReleaseAsset[],
  platform: string,
  arch: string,
): ReleaseAsset | undefined {
  if (platform === 'darwin')
    return ['arm64', 'x64'].includes(arch) ? pickMacAsset(assets, arch) : undefined
  const os = platform === 'win32' ? 'windows' : platform === 'linux' ? 'linux' : ''
  const cpu = arch === 'x64' ? 'x86-64' : arch === 'arm64' ? 'arm64' : ''
  if (!os || !cpu) return undefined
  const name = `stockfish-${os}-${cpu}-universal.${platform === 'win32' ? 'zip' : 'tar.gz'}`
  return assets.find((asset) => asset.name === name)
}
