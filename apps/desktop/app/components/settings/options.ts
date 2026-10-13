import type { MicrophoneStatus, NotificationSkip } from '@kchess/contracts/types'

/** Shared option lists for the settings sections; each section imports what it renders. */
export const appearanceItems = [
  { label: 'System', value: 'system', icon: 'i-lucide-monitor' },
  { label: 'Light', value: 'light', icon: 'i-lucide-sun' },
  { label: 'Dark', value: 'dark', icon: 'i-lucide-moon' },
] as const

export const coordinateItems = [
  { label: 'Inside', value: 'inside' },
  { label: 'Outside', value: 'outside' },
  { label: 'Hidden', value: 'none' },
] as const

export const animationItems = [
  { label: 'Off', value: 'none' },
  { label: 'Fast', value: 'fast' },
  { label: 'Normal', value: 'normal' },
  { label: 'Slow', value: 'slow' },
] as const

export const promotionItems = [
  { label: 'Ask me', value: 'ask' },
  { label: 'Always queen', value: 'queen' },
  { label: 'Queen when premoving', value: 'premove' },
] as const

/** The two pickers on the App theme page: one theme for each appearance. */
export const themePickers = [
  {
    key: 'lightTheme',
    label: 'Light mode theme',
    dark: false,
  },
  { key: 'darkTheme', label: 'Dark mode theme', dark: true },
] as const

export const notifyCategories = [
  {
    key: 'notifyOpponentMove',
    title: 'Opponent moves',
  },
  {
    key: 'notifyLowTime',
    title: 'Low on time',
  },
  {
    key: 'notifyGameEvents',
    title: 'Game start and result',
  },
  {
    key: 'notifyComputerMove',
    title: 'Computer moves',
  },
  {
    key: 'notifyChallenges',
    title: 'Challenges',
  },
] as const

export const pollItems = [
  { label: 'Off', value: 0 },
  { label: '2 min', value: 2 },
  { label: '5 min', value: 5 },
  { label: '15 min', value: 15 },
  { label: '30 min', value: 30 },
  { label: '1 hour', value: 60 },
]

/** Simple yes/no rows of the Gameplay, Online play and Analysis sections. */
export const gameplaySwitches = [
  {
    key: 'showOpeningName',
    title: 'Show opening names',
  },
  {
    key: 'blindfold',
    title: 'Blindfold',
    hint: 'Hide pieces while playing.',
  },
  {
    key: 'zenMode',
    title: 'Zen mode',
    hint: 'Board and game controls only · Z to toggle.',
  },
] as const

export const onlineSwitches = [
  {
    key: 'receiveChallenges',
    title: 'Receive challenges',
  },
  {
    key: 'onlineChat',
    title: 'Game chat',
  },
] as const

export const analysisSwitches = [
  {
    key: 'cloudEval',
    title: 'Lichess cloud evaluation',
    hint: 'Sends analyzed positions to Lichess. Paused during your online games.',
  },
] as const

export const micBadges: Record<
  MicrophoneStatus,
  { label: string; color: 'success' | 'error' | 'neutral' }
> = {
  granted: { label: 'Allowed', color: 'success' },
  denied: { label: 'Blocked', color: 'error' },
  restricted: { label: 'Restricted', color: 'error' },
  'not-determined': { label: 'Not asked yet', color: 'neutral' },
  unknown: { label: 'Unknown', color: 'neutral' },
}

export const skipReasons: Record<NotificationSkip, string> = {
  disabled: 'Notifications are turned off.',
  'category-off': 'That kind of notification is turned off.',
  'window-state': 'Alerts are off for the current window state.',
  unsupported: 'This system does not support desktop notifications.',
  failed: 'The system refused the notification.',
}
