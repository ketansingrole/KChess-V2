//! The Startup folder's shortcut, written through the shell's own link object (`IShellLinkW`),
//! as the installer's Start menu shortcut is.

// The shell link is reached through COM calls, each with its SAFETY comment.
#![allow(
    unsafe_code,
    reason = "COM's shell link calls are unsafe; this module writes KChess's Startup shortcut"
)]

use std::io;
use std::path::Path;

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{HSTRING, Interface as _};

use super::Entry;

/// One argument as `CommandLineToArgvW` reads it back: quoted when it has a space, a tab or a
/// quote, its quotes and the backslashes before them escaped.
fn quoted(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut slashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(slashes * 2 + 1));
                slashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(slashes));
                slashes = 0;
            }
        }
        if c != '\\' {
            out.push(c);
        }
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

/// Write the shortcut to `entry`'s program at `path`.
pub fn write(path: &Path, entry: &Entry) -> io::Result<()> {
    // SAFETY: this thread's own apartment, left before returning when this call joined it.
    let joined = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let written = save(path, entry);
    if joined {
        // SAFETY: balances the successful `CoInitializeEx` above, on the same thread.
        unsafe { CoUninitialize() };
    }
    written.map_err(|cause| io::Error::other(cause.message()))
}

fn save(path: &Path, entry: &Entry) -> windows::core::Result<()> {
    // SAFETY: the shell's own link class; the object is released when dropped.
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }?;
    let arguments = entry
        .args
        .iter()
        .map(|arg| quoted(arg))
        .collect::<Vec<_>>()
        .join(" ");
    // SAFETY: every string outlives the call it is passed to.
    unsafe {
        link.SetPath(&HSTRING::from(entry.program.as_os_str()))?;
        link.SetArguments(&HSTRING::from(arguments))?;
        link.SetDescription(&HSTRING::from(entry.title.as_str()))?;
        if let Some(folder) = entry.program.parent() {
            link.SetWorkingDirectory(&HSTRING::from(folder.as_os_str()))?;
        }
    }
    let file: IPersistFile = link.cast()?;
    // SAFETY: the path outlives the call.
    unsafe { file.Save(&HSTRING::from(path.as_os_str()), true) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn arguments_are_quoted_as_windows_reads_them() {
        assert_eq!(super::quoted("--background"), "--background");
        assert_eq!(super::quoted("a b"), "\"a b\"");
        assert_eq!(super::quoted(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(super::quoted(r"C:\a b\"), r#""C:\a b\\""#);
        assert_eq!(super::quoted(""), "\"\"");
    }
}
