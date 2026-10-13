/** Test-only capture of expected failures as values (golden traces record error messages). */
export function errorText(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause)
}

export type Captured<T> = { value: T } | { error: string }

/** Run `run`, capturing a thrown error as `{ error }` instead of failing the test. */
export function capture<T>(run: () => T): Captured<T> {
  try {
    return { value: run() }
    // eslint-disable-next-line logging/no-silent-catch -- tests intentionally record errors as values to assert failure behavior; shipped code keeps the strict rule.
  } catch (cause) {
    return { error: errorText(cause) }
  }
}

/**
 * Run `run`, capturing a rejection as `{ error }`. Written with a rejection
 * handler (not try/catch) so this helper holds the area's single silent-catch.
 */
export function captureAsync<T>(run: () => Promise<T>): Promise<Captured<T>> {
  return Promise.resolve()
    .then(run)
    .then(
      (value) => ({ value }),
      (cause) => ({ error: errorText(cause) }),
    )
}
