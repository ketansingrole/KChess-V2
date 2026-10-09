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
