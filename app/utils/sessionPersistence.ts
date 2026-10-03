import { onScopeDispose, ref, watch, type Ref } from 'vue'

/** Versioned, bounded local documents; no account credentials or engine output are persisted. */
export function readSession<T>(
  key: string,
  decode: (value: unknown) => T | undefined,
): T | undefined {
  try {
    const text = localStorage.getItem(key)
    if (!text || text.length > 2_000_000) return undefined
    const value: unknown = JSON.parse(text)
    return decode(value)
  } catch {
    return undefined
  }
}
export function persistSession<T>(key: string, source: () => T): Ref<string> {
  const error = ref('')
  let timer: ReturnType<typeof setTimeout> | undefined
  const save = (): void => {
    clearTimeout(timer)
    try {
      const value = JSON.stringify(source())
      if (value.length > 2_000_000)
        throw new Error('This study is too large to save automatically. Export a PGN copy.')
      localStorage.setItem(key, value)
      error.value = ''
    } catch (cause) {
      error.value =
        cause instanceof Error ? cause.message : 'Automatic saving failed. Export a PGN copy.'
    }
  }
  const off = watch(
    source,
    () => {
      clearTimeout(timer)
      timer = setTimeout(save, 150)
    },
    { deep: true },
  )
  window.addEventListener('beforeunload', save)
  onScopeDispose(() => {
    save()
    off()
    window.removeEventListener('beforeunload', save)
  })
  return error
}
