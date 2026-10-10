import { nativeCallSync } from './nativeCore.ts'

/** One asset of a Stockfish GitHub release, as the core picks from it (`crates/kchess-core`). */
export interface ReleaseAsset {
  name: string
  browser_download_url: string
  size: number
  digest?: string | null
}

/** The macOS build for this CPU; the universal binary runs on both architectures. */
export function pickMacAsset(assets: ReleaseAsset[], arch: string): ReleaseAsset | undefined {
  return (
    nativeCallSync<ReleaseAsset | null>('managedEngine.pickMacAsset', assets, arch) ?? undefined
  )
}

/** The asset of a release for `platform` (a Node `process.platform`) and `arch` (`process.arch`). */
export function pickStockfishAsset(
  assets: ReleaseAsset[],
  platform: string,
  arch: string,
): ReleaseAsset | undefined {
  return (
    nativeCallSync<ReleaseAsset | null>('managedEngine.pickAsset', assets, platform, arch) ??
    undefined
  )
}
