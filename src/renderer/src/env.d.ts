/// <reference types="vite/client" />
import type { DesktopApi } from '../../shared/types'
declare global { interface Window { kchess: DesktopApi } }
declare module '*.vue' { import type { DefineComponent } from 'vue'; const component: DefineComponent; export default component }
export {}
