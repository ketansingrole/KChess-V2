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

use kchess_delta::Asset;

use super::{Built, Identity, stage_folder};
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

/// Where an installer takes the program files from.
#[derive(Clone, Copy)]
pub enum Payload<'a> {
    /// Compressed into the installer from this staged folder: the installer people download.
    Inside(&'a Path),
    /// From the `files` folder beside the installer: the update installer, which the updater
    /// runs over the files it rebuilt from an update patch (`crates/kchess-delta`), so the
    /// installation is written, registered and given its uninstaller exactly as by the full one.
    Beside,
}

/// Where packaging leaves the update installer for `cargo xtask deltas`, which puts it in
/// each Windows patch.
pub fn update_installer(identity: &Identity) -> PathBuf {
    root()
        .join("target/package")
        .join(identity.asset(Asset::WindowsUpdateInstaller))
}

/// The uninstaller every installer writes beside product `product` (`Uninstall KChess.exe`, as
/// the Electron app's did): the next installer runs it, and the updater leaves it out of the
/// installation's files.
pub fn uninstaller(product: &str) -> String {
    format!("Uninstall {product}.exe")
}

/// The installer script for the program files of `payload`.
pub fn script(
    identity: &Identity,
    payload: Payload,
    icon: &Path,
    out_file: &Path,
    size_kb: u64,
) -> String {
    let product = identity.name;
    let exe = identity.windows_exe;
    let version = &identity.version;
    let guid = identity.guid;
    let uninstaller = uninstaller(product);
    let files = match payload {
        Payload::Inside(files) => format!(r#"File /r "{}\*.*""#, files.display()),
        // A copy that fails leaves no half-written installation reported as done.
        Payload::Beside => r#"ClearErrors
  CopyFiles /SILENT "$EXEDIR\files\*.*" "$INSTDIR"
  ${If} ${Errors}
    Abort "The update's files could not be copied."
  ${EndIf}"#
            .to_string(),
    };
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
!define UNINSTALLER "{uninstaller}"

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
  {files}
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

/// The scripts of the installer (`installer`, with the staged `folder` inside) and of the
/// update installer (`update_installer`), for `size_kb` of program files.
fn scripts(
    identity: &Identity,
    folder: &Path,
    installer: &Path,
    size_kb: u64,
) -> Vec<(PathBuf, String)> {
    let icon = root().join("apps/gpui/build/icon.ico");
    let update = update_installer(identity);
    vec![
        (
            installer.to_path_buf(),
            script(identity, Payload::Inside(folder), &icon, installer, size_kb),
        ),
        (
            update.clone(),
            script(identity, Payload::Beside, &icon, &update, size_kb),
        ),
    ]
}

pub fn package(
    identity: &Identity,
    built: &Built,
    stage: &Path,
    out: &Path,
) -> Result<Vec<PathBuf>> {
    let folder = stage.join(identity.name);
    stage_folder(built, &folder, identity.windows_exe)?;
    let installer = out.join(identity.asset(Asset::WindowsInstaller));
    let script_path = stage.join("installer.nsi");
    for (file, text) in scripts(identity, &folder, &installer, folder_size(&folder) / 1024) {
        // makensis reads the script as UTF-8 with a byte order mark.
        std::fs::write(&script_path, format!("\u{feff}{text}"))
            .map_err(|cause| cause.to_string())?;
        say(&format!("Making {}…", file.display()));
        run_with(Command::new(makensis()).arg("/V2").arg(&script_path))?;
    }
    Ok(vec![installer])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_final_installer_keeps_the_electron_apps_identity() {
        let identity = Identity::new("2026.10.16".into());
        let text = script(
            &identity,
            Payload::Inside(Path::new("C:/stage/KChess")),
            Path::new("C:/icon.ico"),
            Path::new("C:/out.exe"),
            1234,
        );
        assert!(text.contains(r#"File /r "C:/stage/KChess\*.*""#));
        assert!(!text.contains("CopyFiles /SILENT \"$EXEDIR"));
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

    /// Packaging makes both installers, each told the program files' size.
    #[test]
    fn packaging_makes_the_installer_and_the_update_installer() {
        let identity = Identity::new("2026.11.7".into());
        let made = scripts(
            &identity,
            Path::new("/stage/KChess"),
            Path::new("/out/KChess-2026.11.7-win-x64.exe"),
            4321,
        );
        assert_eq!(made.len(), 2);
        assert_eq!(made[0].0, Path::new("/out/KChess-2026.11.7-win-x64.exe"));
        assert!(
            made[0].1.contains("File /r")
                && made[0]
                    .1
                    .contains("OutFile \"/out/KChess-2026.11.7-win-x64.exe\"")
        );
        assert_eq!(made[1].0, update_installer(&identity));
        assert!(!made[1].1.contains("File /r"));
        assert!(
            made.iter()
                .all(|(_, text)| text.contains("EstimatedSize 4321"))
        );
    }

    /// The update installer is the installer without its files: the same keys, uninstaller and
    /// shortcuts, written over the files beside it, after the earlier installation's uninstaller.
    #[test]
    fn the_update_installer_takes_the_files_beside_it() {
        let identity = Identity::new("2026.11.6".into());
        let full = script(
            &identity,
            Payload::Inside(Path::new("C:/stage/KChess")),
            Path::new("C:/icon.ico"),
            Path::new("C:/out.exe"),
            99,
        );
        let update = script(
            &identity,
            Payload::Beside,
            Path::new("C:/icon.ico"),
            Path::new("C:/out.exe"),
            99,
        );
        assert!(!update.contains("File /r"));
        let copy = update
            .find(r#"CopyFiles /SILENT "$EXEDIR\files\*.*" "$INSTDIR""#)
            .unwrap();
        assert!(update.find("old-uninstaller.exe").unwrap() < copy);
        assert!(copy < update.find("WriteUninstaller").unwrap());
        assert!(update.contains(r#"Abort "The update's files could not be copied.""#));
        let without = |text: &str| -> String {
            text.lines()
                .filter(|line| {
                    !line.contains("File /r")
                        && !line.contains("Errors")
                        && !line.contains("CopyFiles /SILENT \"$EXEDIR")
                        && !line.contains("Abort")
                        && !line.contains("EndIf")
                })
                .collect()
        };
        assert_eq!(without(&full), without(&update));
        assert!(
            update_installer(&identity)
                .ends_with("target/package/KChess-2026.11.6-win-x64-update.exe")
        );
    }
}
