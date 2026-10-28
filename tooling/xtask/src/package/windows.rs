//! Windows: the per-user NSIS installer, made with `makensis` from a script written here.
//!
//! It keeps the Electron app's installation identity, so installing it (or Electron's updater
//! running it with `--updated /S`) upgrades any KChess installation in place: the same GUID
//! (`Software\<guid>` and the per-user uninstall key), the same folder (`%LOCALAPPDATA%\Programs\
//! KChess`, or wherever the installation already is, or `/D=`), the same `KChess.exe`, `Uninstall
//! KChess.exe` and shortcuts. An earlier installation's own uninstaller runs first, in place and
//! keeping the user's data, as electron-builder's installer ran it; an all-users copy that releases
//! up to 2026.10.11 could make is removed too. `--force-run` (or an installation that is not
//! silent) starts KChess afterwards.

#![cfg_attr(unix, allow(dead_code))]

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Built, Identity, copy_program, copy_voice};
use crate::util::{Result, root, run_with, say};

fn folder_size(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                folder_size(&path)
            } else {
                entry.metadata().map_or(0, |meta| meta.len())
            }
        })
        .sum()
}

/// The installer script for the staged folder `files`.
pub fn script(
    identity: &Identity,
    files: &Path,
    icon: &Path,
    out_file: &Path,
    size_kb: u64,
) -> String {
    let product = identity.name;
    let exe = identity.windows_exe;
    let version = &identity.version;
    let guid = identity.guid;
    let files = files.display();
    let icon = icon.display();
    let out_file = out_file.display();
    let copyright = super::COPYRIGHT;
    let author = super::AUTHOR;
    // NSIS needs a numeric four-part version for VIProductVersion.
    let mut numbers: Vec<String> = version.split('.').map(str::to_string).collect();
    numbers.resize(4, "0".into());
    let numeric = numbers.join(".");
    format!(
        r#"; Written by `cargo xtask package` (tooling/xtask/src/package/windows.rs).
Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!define PRODUCT "{product}"
!define EXE "{exe}.exe"
!define VERSION "{version}"
!define INSTALL_KEY "Software\{guid}"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\{guid}"
!define UNINSTALLER "Uninstall {product}.exe"

!include "FileFunc.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

Name "${{PRODUCT}}"
OutFile "{out_file}"
Icon "{icon}"
UninstallIcon "{icon}"
InstallDir "$LOCALAPPDATA\Programs\${{PRODUCT}}"
ShowInstDetails nevershow
ShowUninstDetails nevershow
AutoCloseWindow true
BrandingText "${{PRODUCT}} ${{VERSION}}"
VIProductVersion "{numeric}"
VIAddVersionKey "ProductName" "${{PRODUCT}}"
VIAddVersionKey "ProductVersion" "${{VERSION}}"
VIAddVersionKey "FileVersion" "${{VERSION}}"
VIAddVersionKey "FileDescription" "${{PRODUCT}} installer"
VIAddVersionKey "CompanyName" "{author}"
VIAddVersionKey "LegalCopyright" "{copyright}"

Page instfiles
UninstPage instfiles

Function .onInit
  ; The app is 64-bit: its keys, and the ones every earlier installer wrote, are in the 64-bit view.
  ${{If}} ${{RunningX64}}
    SetRegView 64
  ${{EndIf}}
  ; An installation's own folder, unless /D= names another.
  ReadRegStr $R0 HKCU "${{INSTALL_KEY}}" InstallLocation
  ClearErrors
  ${{GetOptions}} $CMDLINE "/D=" $R1
  ${{If}} $R0 != ""
  ${{AndIf}} ${{Errors}}
    StrCpy $INSTDIR $R0
  ${{EndIf}}
FunctionEnd

Section "Install"
  SetOutPath "$INSTDIR"
  ; A running copy keeps its files open.
  nsExec::Exec 'taskkill /F /IM "${{EXE}}"'
  Sleep 500
  ; The earlier installation's uninstaller (the Electron app's or this app's), run in place and
  ; keeping the user's data, before any new file is written.
  ${{If}} ${{FileExists}} "$INSTDIR\${{UNINSTALLER}}"
    CopyFiles /SILENT /FILESONLY "$INSTDIR\${{UNINSTALLER}}" "$PLUGINSDIR\old-uninstaller.exe"
    ExecWait '"$PLUGINSDIR\old-uninstaller.exe" /S /KEEP_APP_DATA --updated _?=$INSTDIR' $R1
    DetailPrint "Removed the earlier installation in $INSTDIR (exit code $R1)."
  ${{EndIf}}
  ; Releases up to 2026.10.11 could install for all users; this per-user installation replaces it.
  ReadRegStr $R0 HKLM "${{INSTALL_KEY}}" InstallLocation
  ${{If}} $R0 == ""
    ReadRegStr $R0 HKLM "${{UNINSTALL_KEY}}" InstallLocation
  ${{EndIf}}
  ${{If}} $R0 != ""
  ${{AndIf}} ${{FileExists}} "$R0\${{UNINSTALLER}}"
    CopyFiles /SILENT /FILESONLY "$R0\${{UNINSTALLER}}" "$PLUGINSDIR\all-users-uninstaller.exe"
    ExecWait '"$PLUGINSDIR\all-users-uninstaller.exe" /S /allusers --updated _?=$R0' $R1
    DetailPrint "Removed the all-users installation in $R0 (exit code $R1)."
  ${{EndIf}}
  SetOutPath "$INSTDIR"
  File /r "{files}\*.*"
  WriteUninstaller "$INSTDIR\${{UNINSTALLER}}"

  WriteRegStr HKCU "${{INSTALL_KEY}}" InstallLocation "$INSTDIR"
  WriteRegStr HKCU "${{UNINSTALL_KEY}}" DisplayName "${{PRODUCT}} ${{VERSION}}"
  WriteRegStr HKCU "${{UNINSTALL_KEY}}" DisplayVersion "${{VERSION}}"
  WriteRegStr HKCU "${{UNINSTALL_KEY}}" Publisher "{author}"
  WriteRegStr HKCU "${{UNINSTALL_KEY}}" DisplayIcon "$INSTDIR\${{EXE}},0"
  WriteRegStr HKCU "${{UNINSTALL_KEY}}" InstallLocation "$INSTDIR"
  WriteRegStr HKCU "${{UNINSTALL_KEY}}" UninstallString '"$INSTDIR\${{UNINSTALLER}}" /currentuser'
  WriteRegStr HKCU "${{UNINSTALL_KEY}}" QuietUninstallString '"$INSTDIR\${{UNINSTALLER}}" /currentuser /S'
  WriteRegDWORD HKCU "${{UNINSTALL_KEY}}" EstimatedSize {size_kb}
  WriteRegDWORD HKCU "${{UNINSTALL_KEY}}" NoModify 1
  WriteRegDWORD HKCU "${{UNINSTALL_KEY}}" NoRepair 1

  CreateShortCut "$SMPROGRAMS\${{PRODUCT}}.lnk" "$INSTDIR\${{EXE}}" "" "$INSTDIR\${{EXE}}" 0
  CreateShortCut "$DESKTOP\${{PRODUCT}}.lnk" "$INSTDIR\${{EXE}}" "" "$INSTDIR\${{EXE}}" 0

  ; The updater asks for the app to start again; a person installing by hand sees it start.
  StrCpy $R3 "0"
  ClearErrors
  ${{GetOptions}} $CMDLINE "--force-run" $R2
  ${{IfNot}} ${{Errors}}
    StrCpy $R3 "1"
  ${{EndIf}}
  ${{IfNot}} ${{Silent}}
    StrCpy $R3 "1"
  ${{EndIf}}
  ${{If}} $R3 == "1"
    Exec '"$INSTDIR\${{EXE}}"'
  ${{EndIf}}
SectionEnd

Function un.onInit
  ${{If}} ${{RunningX64}}
    SetRegView 64
  ${{EndIf}}
FunctionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /F /IM "${{EXE}}"'
  Sleep 500
  RMDir /r "$INSTDIR"
  ; An upgrade's new installer writes its own shortcuts and keys at once.
  ClearErrors
  ${{GetOptions}} $CMDLINE "--updated" $R0
  ${{If}} ${{Errors}}
    Delete "$SMPROGRAMS\${{PRODUCT}}.lnk"
    Delete "$DESKTOP\${{PRODUCT}}.lnk"
  ${{EndIf}}
  DeleteRegKey HKCU "${{UNINSTALL_KEY}}"
  DeleteRegKey HKCU "${{INSTALL_KEY}}"
SectionEnd
"#
    )
}

fn makensis() -> PathBuf {
    let installed = PathBuf::from(r"C:\Program Files (x86)\NSIS\makensis.exe");
    if installed.exists() {
        installed
    } else {
        PathBuf::from("makensis")
    }
}

pub fn package(
    identity: &Identity,
    built: &Built,
    stage: &Path,
    out: &Path,
) -> Result<Vec<PathBuf>> {
    let folder = stage.join(identity.name);
    copy_program(
        &built.app,
        &folder.join(format!("{}.exe", identity.windows_exe)),
    )?;
    copy_program(&built.stockfish, &folder.join("stockfish.exe"))?;
    copy_program(&built.service, &folder.join("kchess-service.exe"))?;
    copy_program(&built.cli, &folder.join("kchess-cli.exe"))?;
    copy_voice(built, &folder)?;
    std::fs::copy(&built.licenses, folder.join("THIRD_PARTY_LICENSES.txt"))
        .map_err(|cause| cause.to_string())?;
    let installer = out.join(format!(
        "{}-{}-win-x64.exe",
        identity.prefix(),
        identity.version
    ));
    let script_path = stage.join("installer.nsi");
    let text = script(
        identity,
        &folder,
        &root().join("apps/gpui/build/icon.ico"),
        &installer,
        folder_size(&folder) / 1024,
    );
    // makensis reads the script as UTF-8 with a byte order mark.
    std::fs::write(&script_path, format!("\u{feff}{text}")).map_err(|cause| cause.to_string())?;
    say("Making the installer…");
    run_with(Command::new(makensis()).arg("/V2").arg(&script_path))?;
    Ok(vec![installer])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_final_installer_keeps_the_electron_apps_identity() {
        let identity = Identity::new(true, "2026.10.16".into());
        let text = script(
            &identity,
            Path::new("C:/stage/KChess"),
            Path::new("C:/icon.ico"),
            Path::new("C:/out.exe"),
            1234,
        );
        assert!(text.contains(r#"!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\6bb66f38-a1a5-5ec8-b24f-c899466885ca""#));
        assert!(
            text.contains(r#"!define INSTALL_KEY "Software\6bb66f38-a1a5-5ec8-b24f-c899466885ca""#)
        );
        assert!(text.contains(r#"!define EXE "KChess.exe""#));
        assert!(text.contains(r#"!define UNINSTALLER "Uninstall KChess.exe""#));
        assert!(text.contains(r#"InstallDir "$LOCALAPPDATA\Programs\${PRODUCT}""#));
        assert!(text.contains("VIProductVersion \"2026.10.16.0\""));
        assert!(text.contains("EstimatedSize 1234"));
        assert!(!text.contains("{{"), "no unexpanded braces");
        // A 32-bit installer reads the 64-bit registry view, where earlier installers wrote.
        assert_eq!(text.matches("SetRegView 64").count(), 2);
    }

    #[test]
    fn the_preview_installs_apart() {
        let text = script(
            &Identity::new(false, "2026.10.16".into()),
            Path::new("s"),
            Path::new("i"),
            Path::new("o"),
            1,
        );
        assert!(text.contains("3ddfc8df-572e-41e0-a3bd-fc08e5e3da9f"));
        assert!(!text.contains("6bb66f38"));
        assert!(text.contains(r#"!define EXE "KChess Preview.exe""#));
    }
}
