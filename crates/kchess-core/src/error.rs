//! Errors as they cross to the host: a message, and whether it was a cancellation. They are the
//! contracts' (`kchess_contracts::error`), so a frontend reads them without depending on the core.

pub use kchess_contracts::error::{CoreError, ErrorKind, Result};
