; Releases up to 2026.10.11 used an assisted installer that could install KChess for all users.
; The one-click installer is per-user and replaces only a per-user copy, so it also removes an
; all-users copy. That copy's uninstaller asks for administrator rights.
!macro customInstall
  ReadRegStr $R0 HKLM "${INSTALL_REGISTRY_KEY}" InstallLocation
  ${if} $R0 != ""
  ${andIf} ${FileExists} "$R0\Uninstall ${PRODUCT_FILENAME}.exe"
    CopyFiles /SILENT /FILESONLY "$R0\Uninstall ${PRODUCT_FILENAME}.exe" "$PLUGINSDIR\all-users-uninstaller.exe"
    ExecWait '"$PLUGINSDIR\all-users-uninstaller.exe" /S /allusers --updated _?=$R0' $R1
    DetailPrint "Removed the all-users installation in $R0 (exit code $R1)."
  ${endIf}
!macroend
