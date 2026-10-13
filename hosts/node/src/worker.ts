import {
  validateCoreArguments,
  createKChessCore,
  CORE_METHODS,
  type CoreMethod,
  type KChessCore,
} from '@kchess/native'
import { parentPort, workerData } from 'node:worker_threads'
import { errorSummary, logWarn } from '@kchess/native/logger'
import { nodePlatform, type NodeCoreOptions } from './platform'
import type { HostRequest, HostResponse } from './protocol'
import { HOST_METHODS, validateHostArguments, type HostMethod } from './hostArguments'

if (!parentPort) throw new Error('The Node core host requires a worker.')
const port = parentPort
const send = (message: HostResponse): void => port.postMessage(message)
try {
  const core = createKChessCore(await nodePlatform(workerData as NodeCoreOptions))
  const events = [
    'online:state',
    'online:event',
    'online:error',
    'puzzledb:progress',
    'engine:analysis',
    'review:update',
    'review:status',
    'challenges:update',
    'online:lobby',
    'online:ongoing-changed',
    'watch:state',
    'watch:frame',
    'watch:broadcast',
    'challenge:received',
    'settings:saved',
  ] as const
  for (const event of events) core.on(event, (payload) => send({ event, payload }))
  const methods = new Set<string>([...CORE_METHODS, ...HOST_METHODS])
  port.on('message', async ({ id, method, args }: HostRequest) => {
    try {
      if (!Number.isSafeInteger(id) || !methods.has(method) || !Array.isArray(args))
        throw new Error('Invalid Node core request.')
      if ((CORE_METHODS as readonly string[]).includes(method))
        validateCoreArguments(method as CoreMethod, args)
      else validateHostArguments(method as HostMethod, args)
      const call = core[method as keyof KChessCore] as (...args: unknown[]) => unknown
      const result = await call(...args)
      send({ id, result })
      if (method === 'close') port.close()
    } catch (cause) {
      logWarn('node-core', 'Core request failed:', `method=${method}`, errorSummary(cause))
      send({ id, error: cause instanceof Error ? cause.message : 'Core request failed.' })
    }
  })
  send({ ready: true })
} catch (cause) {
  logWarn('node-core', 'Core startup failed:', errorSummary(cause))
  send({ startupError: cause instanceof Error ? cause.message : 'Core startup failed.' })
  port.close()
}
