export const PERFORMANCE_BUDGETS = Object.freeze({
  rendererBytes: 12 * 1024 * 1024,
  packageBytes: 32 * 1024 * 1024,
  individualFileBytes: 16 * 1024 * 1024,
  // The speech recognition runtime (the helper and the official prebuilt libvosk beside it),
  // budgeted apart from the app payload because its size is upstream's. Measured 2026-10-21:
  // libvosk 0.3.42's macOS arm64 slice 5.6 MiB; 0.3.45 Linux x64 24.8 MiB; 0.3.45 Windows x64
  // 25.2 MiB plus its MinGW runtime (libstdc++ 25.4 MiB, libgcc 0.6 MiB, winpthread 0.4 MiB);
  // the helper about 1 MiB.
  voiceRuntimeBytes: 56 * 1024 * 1024,
  libraryPageRows: 100,
  // Representative short-game fixture; full PGNs must never be included in list rows.
  libraryFixturePageBytes: 64 * 1024,
})

export function assertLibraryBudget(sample) {
  if (sample.pageRows !== PERFORMANCE_BUDGETS.libraryPageRows)
    throw new Error('Library benchmark did not return a full bounded page.')
  if (sample.pageSerializedBytes > PERFORMANCE_BUDGETS.libraryFixturePageBytes)
    throw new Error('Representative library page exceeds the 64 KiB transfer budget.')
}
