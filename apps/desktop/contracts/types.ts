import type {
  AnalysisUpdate,
  AppData,
  AppUpdateStatus,
  BroadcastUpdate,
  ChallengeInfo,
  CoreApi,
  CustomThemeReport,
  ExportRequest,
  LobbyState,
  MicrophoneAccess,
  NotificationRequest,
  NotificationResult,
  OAuthPageLook,
  OnlineConnection,
  OnlineEvent,
  PuzzleDbProgress,
  ReviewStatus,
  ReviewUpdate,
  Settings,
  VoiceModelProgress,
  VoiceModelStatus,
  WatchFrame,
  WatchState,
} from '../../../core/src/contracts/types'
import type { PerformanceName, RendererErrorReport } from './rendererDiagnostics'
export * from '../../../core/src/contracts/types'

/** The desktop frontend: the core plus window, OS integration and event subscriptions. */
export interface DesktopApi extends CoreApi {
  loadData(): Promise<AppData>
  saveSettings(settings: Settings): Promise<Settings>
  addAccount(username: string): Promise<AppData>
  logout(username: string): Promise<AppData>
  logoutAll(): Promise<AppData>
  removeAccount(username: string): Promise<AppData>
  syncGames(username?: string): Promise<AppData>
  clearAccountData(username: string): Promise<AppData>
  addFriends(names: string[]): Promise<AppData>
  connectLichess(look?: OAuthPageLook): Promise<{ data: AppData; username: string }>
  /** Flush pending frontend writes before the shell closes the core. */
  onPrepareQuit(callback: (token: string) => void): () => void
  completeQuit(token: string): Promise<void>
  reportRendererError(report: RendererErrorReport): Promise<void>
  recordPerformance(name: PerformanceName, milliseconds: number): Promise<void>
  /**
   * Save an exported game (an animated GIF, a PNG of a position, or PGN text) where the native
   * dialog says; false when cancelled.
   */
  saveExport(request: ExportRequest): Promise<boolean>
  /** Save redacted runtime diagnostics to a location chosen in the native dialog. */
  exportDiagnostics(): Promise<boolean>
  /** Frameless-chrome window controls (Windows/Linux); no-ops where the OS draws its own chrome. */
  windowMinimize(): Promise<void>
  windowToggleMaximize(): Promise<{ maximized: boolean }>
  windowClose(): Promise<void>
  windowIsMaximized(): Promise<boolean>
  onWindowMaximized(callback: (state: { maximized: boolean }) => void): () => void
  appUpdateStatus(): Promise<AppUpdateStatus>
  checkAppUpdate(): Promise<AppUpdateStatus>
  downloadAppUpdate(): Promise<AppUpdateStatus>
  installAppUpdate(): Promise<void>
  /** Opens only this app's official GitHub Releases page. */
  openAppReleases(): Promise<void>
  onAppUpdate(callback: (status: AppUpdateStatus) => void): () => void
  chooseEngine(): Promise<string | null>
  onAnalysis(callback: (update: AnalysisUpdate) => void): () => void
  onReviewUpdate(callback: (update: ReviewUpdate) => void): () => void
  onReviewStatus(callback: (status: ReviewStatus) => void): () => void
  onChallenges(callback: (challenges: ChallengeInfo[]) => void): () => void
  onLobbyState(callback: (state: LobbyState) => void): () => void
  onWatchState(callback: (state: WatchState) => void): () => void
  onWatch(callback: (frame: WatchFrame) => void): () => void
  onBroadcast(callback: (update: BroadcastUpdate) => void): () => void
  /** A game started or ended that the board does not show (refresh `ongoingGames`). */
  onOngoingChanged(callback: () => void): () => void
  /** Show a desktop notification if Settings allow it for this kind and the window's state. */
  notify(request: NotificationRequest): Promise<NotificationResult>
  /** Custom themes from the themes folder (created on first use). */
  loadThemes(): Promise<CustomThemeReport>
  openThemesFolder(): Promise<void>
  /** Open the operating system's notification settings (macOS and Windows); false where unsupported. */
  openNotificationSettings(): Promise<boolean>
  /** The microphone permission; with `request`, asks the OS (shows its prompt) when undecided. */
  microphoneAccess(request: boolean): Promise<MicrophoneAccess>
  /** Downloads the verified English model on first use, then reuses the local cache. */
  ensureVoiceModel(): Promise<string>
  /** Checks the verified local voice model without downloading or requesting microphone access. */
  voiceModelStatus(): Promise<VoiceModelStatus>
  onVoiceModelProgress(callback: (progress: VoiceModelProgress) => void): () => void
  /** Open the operating system's microphone privacy settings; false where unsupported. */
  openMicrophoneSettings(): Promise<boolean>
  /** Alerts for the window to show itself, sent instead of a system notification while KChess is in use. */
  onNotification(callback: (alert: { title: string; body: string }) => void): () => void
  onOnlineState(callback: (state: OnlineConnection) => void): () => void
  onOnlineEvent(callback: (event: OnlineEvent) => void): () => void
  onOnlineError(callback: (message: string) => void): () => void
  onPuzzleDbProgress(callback: (progress: PuzzleDbProgress) => void): () => void
  /** Save the whole log as JSON where the native dialog says; false when cancelled. */
  exportVoiceHistory(): Promise<boolean>
}
