; The per-user one-click installer would name a new installation folder after the package
; (kchess-electron). Use KChess, as releases with the assisted installer did; an existing
; installation and /D= keep their folder.
!macro customInit
  ReadRegStr $R0 HKCU "${INSTALL_REGISTRY_KEY}" InstallLocation
  !insertmacro GetDParameter $R1
  ${if} $R0 == ""
  ${andIf} $R1 == ""
    ${GetParent} $INSTDIR $R0
    StrCpy $INSTDIR "$R0\${PRODUCT_FILENAME}"
  ${endIf}
!macroend

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
