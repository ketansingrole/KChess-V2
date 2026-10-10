/**
 * The Rust rules (`crates/kchess-domain`), as every runtime reaches them: the core loads them as
 * a Node module (`@kchess/native`), the renderer as WebAssembly (`crates/kchess-wasm`). Both
 * expose one call, `invoke(method, argsJson) → resultJson`, so the domain functions that wrap it
 * are the same everywhere. A host sets its binding once at startup, before any rule is used.
 */
export interface RulesBinding {
  /** Throws with the rules' message when the arguments are malformed. */
  invoke(method: string, args: string): string
}

let binding: RulesBinding | undefined

export function setRulesBinding(next: RulesBinding): void {
  binding = next
}

export function hasRulesBinding(): boolean {
  return binding !== undefined
}

/** Call a rules method; `null` results come back as `null`. */
export function rules<T>(method: string, ...args: unknown[]): T {
  if (!binding) throw new Error('The chess rules are not loaded yet.')
  return JSON.parse(binding.invoke(method, JSON.stringify(args))) as T
}

/** Where a value JSON cannot carry sat: `at` is the argument, then the path inside it. */
interface LosslessSpecial {
  at: (string | number)[]
  /** 0 undefined, 1 NaN, 2 Infinity, 3 -Infinity, 4 a `Uint8Array` (`n` its length, `h` its first bytes). */
  k: number
  /** For an `undefined` property: its index among the object's keys, so it keeps its place. */
  i?: number
  n?: number
  h?: number[]
}

const LONE_SURROGATE = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/
const LONE_SURROGATES = new RegExp(LONE_SURROGATE.source, 'g')

function losslessValue(
  value: unknown,
  at: (string | number)[],
  specials: LosslessSpecial[],
): unknown {
  if (value === undefined) {
    specials.push({ at, k: 0 })
    return null
  }
  if (typeof value === 'number') {
    if (Number.isNaN(value)) specials.push({ at, k: 1 })
    else if (value === Infinity) specials.push({ at, k: 2 })
    else if (value === -Infinity) specials.push({ at, k: 3 })
    else return value
    return null
  }
  if (typeof value === 'string')
    // JSON cannot carry an unpaired surrogate; both halves are one UTF-16 unit, so U+FFFD keeps lengths.
    return LONE_SURROGATE.test(value) ? value.replace(LONE_SURROGATES, '�') : value
  if (value instanceof Uint8Array) {
    specials.push({ at, k: 4, n: value.length, h: Array.from(value.subarray(0, 4)) })
    return null
  }
  if (Array.isArray(value))
    return value.map((item, index) => losslessValue(item, [...at, index], specials))
  if (value !== null && typeof value === 'object') {
    const out: Record<string, unknown> = {}
    const source = value as Record<string, unknown>
    Object.keys(source).forEach((key, index) => {
      const safeKey = LONE_SURROGATE.test(key) ? key.replace(LONE_SURROGATES, '�') : key
      const item = source[key]
      if (item === undefined) specials.push({ at: [...at, safeKey], k: 0, i: index })
      else out[safeKey] = losslessValue(item, [...at, safeKey], specials)
    })
    return out
  }
  return value
}

/**
 * Call a rules method whose arguments are JS values that JSON cannot carry losslessly
 * (`undefined` properties, NaN, infinities, binary data, unpaired surrogates). The Rust side reads
 * a sidecar of where those sat before the JSON arguments, so each check sees what the TypeScript
 * check saw.
 */
export function rulesLossless<T>(method: string, ...args: unknown[]): T {
  if (!binding) throw new Error('The chess rules are not loaded yet.')
  const specials: LosslessSpecial[] = []
  const safe = args.map((arg, index) => losslessValue(arg, [index], specials))
  return JSON.parse(binding.invoke(method, JSON.stringify([specials, ...safe]))) as T
}

/** The exports of the rules' WebAssembly module (see `crates/kchess-wasm/src/lib.rs`). */
interface RulesExports {
  /** Only its current buffer is read, so the core needs no WebAssembly types. */
  memory: { buffer: ArrayBuffer }
  kc_exp(x: number, fused: number): number
  kc_use_fused_exp(fused: number): void
  kc_alloc(len: number): number
  kc_free(ptr: number, len: number): void
  kc_invoke(method: number, methodLen: number, args: number, argsLen: number): bigint
}

/**
 * Review figures go through `exp`, which must match this engine's `Math.exp` to the bit. V8
 * computes it with fused multiply-adds on some CPUs and not others, which WebAssembly cannot
 * know, so compare both ways with `Math.exp` and keep the one that agrees.
 */
function calibrateExp(wasm: RulesExports): void {
  let fused = 0
  let plain = 0
  for (let i = 1; i <= 2000; i++) {
    const x = -0.00368208 * (i - 1000) - i * 1e-7
    const expected = Math.exp(x)
    if (Object.is(wasm.kc_exp(x, 1), expected)) fused++
    if (Object.is(wasm.kc_exp(x, 0), expected)) plain++
  }
  wasm.kc_use_fused_exp(fused > plain ? 1 : 0)
}

/** A binding over an instantiated rules module. */
export function wasmBinding(exports: Record<string, unknown>): RulesBinding {
  const wasm = exports as unknown as RulesExports
  calibrateExp(wasm)
  const encoder = new TextEncoder()
  const decoder = new TextDecoder()
  const write = (text: string): [number, number] => {
    const bytes = encoder.encode(text)
    const ptr = wasm.kc_alloc(bytes.length)
    new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes)
    return [ptr, bytes.length]
  }
  return {
    invoke(method, args) {
      const [methodPtr, methodLen] = write(method)
      const [argsPtr, argsLen] = write(args)
      try {
        const packed = wasm.kc_invoke(methodPtr, methodLen, argsPtr, argsLen)
        const ptr = Number(packed >> 32n)
        const len = Number(packed & 0xffffffffn)
        // Memory may have grown during the call; read through a fresh view.
        const out = new Uint8Array(wasm.memory.buffer, ptr, len)
        const failed = out[0] === 1
        const text = decoder.decode(out.subarray(1))
        wasm.kc_free(ptr, len)
        if (failed) throw new Error(text)
        return text
      } finally {
        wasm.kc_free(methodPtr, methodLen)
        wasm.kc_free(argsPtr, argsLen)
      }
    },
  }
}
