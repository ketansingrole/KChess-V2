//! Child processes the core starts (the engine, its probe, the voice helper) are console
//! programs. On Windows one started by a process with no console of its own, such as the
//! detached service, gets a new console window; `no_window` keeps it hidden.

use tokio::process::Command;

/// Starts `command` without a console window on Windows; elsewhere it is unchanged.
pub(crate) fn no_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}
