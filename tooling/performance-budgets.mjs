export const PERFORMANCE_BUDGETS = Object.freeze({
  rendererBytes: 12 * 1024 * 1024,
  packageBytes: 32 * 1024 * 1024,
  individualFileBytes: 16 * 1024 * 1024,
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
