//! The release version every part of the app reports (`KCHESS_VERSION`), and on Windows the
//! executable's own resources: the icon Start menu and desktop shortcuts show, and the version
//! Windows reads from the file, as electron-builder wrote them into the Electron app's.

use std::path::Path;

/// The name Windows shows for the executable, the Electron app's `productName`.
const PRODUCT: &str = "KChess";
const AUTHOR: &str = "Ketan Singrole";
const COPYRIGHT: &str = "Copyright © 2026 Ketan Singrole. Licensed under the GPL-3.0-or-later.";

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // The workspace version is the release; test installers are built above every release
    // (`windows-upgrade.yml`) without editing it.
    println!("cargo:rerun-if-env-changed=KCHESS_PACKAGE_VERSION");
    let version = std::env::var("KCHESS_PACKAGE_VERSION")
        .ok()
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=KCHESS_VERSION={version}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources(root, &version);
    }
}

fn windows_resources(root: &Path, version: &str) {
    let icon = root.join("build/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    // 2026.10.14 → 2026,10,14,0.
    let mut parts: Vec<u16> = version
        .split(['.', '-', '+'])
        .map_while(|part| part.parse().ok())
        .collect();
    parts.resize(4, 0);
    let numbers = parts
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let product = PRODUCT;
    let rc = format!(
        // Numbers, not <winver.h>'s names, so no SDK include path is needed: VS_VERSION_INFO is
        // 1, VOS_NT_WINDOWS32 0x40004 and VFT_APP 1.
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "{company}"
      VALUE "FileDescription", "{product}"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "{product}"
      VALUE "LegalCopyright", "{copyright}"
      VALUE "OriginalFilename", "{product}.exe"
      VALUE "ProductName", "{product}"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = icon.display().to_string().replace('\\', "/"),
        company = AUTHOR,
        copyright = COPYRIGHT,
    );
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("kchess.rc");
    std::fs::write(&out, rc).expect("the resource script");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .expect("the Windows resources compile");
}
