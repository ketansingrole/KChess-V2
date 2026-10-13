/** Tooling-only capture of expected failures as values (benchmark records, smoke probes). */
export function errorText(cause) {
  return String(cause?.message ?? cause)
}

/**
 * Run `fn`, capturing a thrown error as `{ error }` instead of failing the script.
 * Tooling probes record failures into reports; shipped code keeps the strict rule.
 */
export function attempt(fn) {
  try {
    return { value: fn() }
    // eslint-disable-next-line logging/no-silent-catch -- tooling intentionally records errors as values in benchmark/smoke reports; shipped code keeps the strict rule.
  } catch (cause) {
    return { error: errorText(cause) }
  }
}

/**
 * Run `fn`, capturing a rejection as `{ error }`. Written with a rejection
 * handler (not try/catch) so this helper holds the area's single silent-catch.
 */
export function attemptAsync(fn) {
  return Promise.resolve()
    .then(fn)
    .then(
      (value) => ({ value }),
      (cause) => ({ error: errorText(cause) }),
    )
}
