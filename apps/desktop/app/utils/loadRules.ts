/** Stream compiled rules when the host serves WASM, avoiding a retained source ArrayBuffer. */
export async function instantiateRules(response: Response): Promise<WebAssembly.Instance> {
  if (!response.ok) throw new Error(`The chess rules could not be loaded (${response.status}).`)
  const mime = response.headers.get('content-type')?.split(';', 1)[0]?.trim().toLowerCase()
  const result =
    mime === 'application/wasm'
      ? await WebAssembly.instantiateStreaming(response)
      : await WebAssembly.instantiate(await response.arrayBuffer())
  return result.instance
}
