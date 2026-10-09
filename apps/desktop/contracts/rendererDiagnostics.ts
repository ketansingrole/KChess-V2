/** Only fixed page paths enter diagnostics; queries can contain account names or game data. */
export const RENDERER_ROUTES = [
  '/',
  '/online',
  '/tournaments',
  '/watch',
  '/local',
  '/computer',
  '/analysis',
  '/studies',
  '/editor',
  '/puzzles',
  '/practice',
  '/history',
  '/insights',
  '/friends',
  '/players',
  '/settings',
] as const
export type RendererRoute = (typeof RENDERER_ROUTES)[number]
export const PERFORMANCE_NAMES = [
  'app.ready',
  'board.frame',
  'voice.activation',
  ...RENDERER_ROUTES.map((route) => `page.navigation:${route}` as const),
] as const
export type PerformanceName = (typeof PERFORMANCE_NAMES)[number]
export interface RendererErrorReport {
  route: RendererRoute
  message: string
  info: string
}
export function diagnosticRoute(path: string): RendererRoute {
  return RENDERER_ROUTES.find((route) => route === path) ?? '/'
}
