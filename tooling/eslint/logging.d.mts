export const rules: Record<
  'no-silent-catch' | 'no-silent-promise-catch' | 'no-raw-console',
  { meta: unknown; create: (context: unknown) => Record<string, unknown> }
>
declare const plugin: { rules: typeof rules }
export default plugin
