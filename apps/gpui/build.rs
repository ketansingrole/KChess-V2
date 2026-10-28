//! The release version every part of the app reports (`KCHESS_VERSION`), and on Windows the
//! executable's own resources: Electron's icon, which Start menu and desktop shortcuts show, and
//! the version Windows reads from the file, as electron-builder writes them into Electron's.

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let package = root.join("package.json");
    println!("cargo:rerun-if-changed={}", package.display());
    // Test installers are built above every release (`windows-upgrade.yml`) without editing
    // package.json, which the lockfile check would reject.
    println!("cargo:rerun-if-env-changed=KCHESS_PACKAGE_VERSION");
    let version = std::env::var("KCHESS_PACKAGE_VERSION")
        .ok()
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| {
            let text = std::fs::read_to_string(&package).expect("the root package.json");
            let json: serde_json::Value = serde_json::from_str(&text).expect("package.json");
            json["version"].as_str().expect("its version").to_string()
        });
    println!("cargo:rustc-env=KCHESS_VERSION={version}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources(&root, &version);
    }
}

fn windows_resources(root: &Path, version: &str) {
    let desktop: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("apps/desktop/package.json"))
            .expect("apps/desktop/package.json"),
    )
    .expect("apps/desktop/package.json");
    let build = &desktop["build"];
    let text = |value: &serde_json::Value| value.as_str().unwrap_or_default().replace('"', "\"\"");
    let icon = root.join("apps/desktop/build/icon.ico");
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
    let product = text(&build["productName"]);
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
        company = text(&desktop["author"]),
        copyright = text(&build["copyright"]),
    );
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("kchess.rc");
    std::fs::write(&out, rc).expect("the resource script");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .expect("the Windows resources compile");
}
