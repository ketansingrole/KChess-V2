//! The local connection between clients and the service: a Unix domain socket on macOS and
//! Linux, a named pipe on Windows. A Unix connection is accepted only from this user
//! (`docs/shared-core.md`, "Who may connect"); the token in `hello` covers both.

use std::path::Path;

use tokio::io::{AsyncRead, AsyncWrite};

/// A connection, whichever transport carries it.
pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub type Stream = Box<dyn Io>;

#[cfg(unix)]
mod imp {
    use super::*;
    use tokio::net::{UnixListener, UnixStream};

    pub struct Listener {
        inner: UnixListener,
        uid: u32,
    }

    impl Listener {
        /// Listens at `path`. The caller holds the profile's lock, so a socket file left by a
        /// core that died is removed first.
        pub fn bind(path: &Path) -> std::io::Result<Listener> {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let inner = UnixListener::bind(path)?;
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            }
            Ok(Listener {
                inner,
                uid: rustix::process::getuid().as_raw(),
            })
        }

        /// The next connection, and whether it comes from this user.
        pub async fn accept(&self) -> std::io::Result<(Stream, bool)> {
            let (stream, _) = self.inner.accept().await?;
            let same_user = stream
                .peer_cred()
                .is_ok_and(|credentials| credentials.uid() == self.uid);
            Ok((Box::new(stream), same_user))
        }
    }

    pub async fn connect(path: &Path) -> std::io::Result<Stream> {
        Ok(Box::new(UnixStream::connect(path).await?))
    }

    /// Removes the socket file when the service closes.
    pub fn remove(path: &Path) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::Mutex;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    pub struct Listener {
        name: String,
        /// The pipe instance waiting for the next client.
        next: Mutex<Option<NamedPipeServer>>,
    }

    impl Listener {
        pub fn bind(path: &Path) -> std::io::Result<Listener> {
            let name = path.to_string_lossy().to_string();
            let first = ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(&name)?;
            Ok(Listener {
                name,
                next: Mutex::new(Some(first)),
            })
        }

        /// The next connection. Named pipes carry no peer user here; the token in `hello` is
        /// the check, and remote clients are rejected.
        pub async fn accept(&self) -> std::io::Result<(Stream, bool)> {
            let server = self
                .next
                .lock()
                .map_err(|_| std::io::Error::other("the pipe is unavailable"))?
                .take();
            let server = match server {
                Some(server) => server,
                None => ServerOptions::new()
                    .reject_remote_clients(true)
                    .create(&self.name)?,
            };
            server.connect().await?;
            let following = ServerOptions::new()
                .reject_remote_clients(true)
                .create(&self.name)?;
            if let Ok(mut next) = self.next.lock() {
                *next = Some(following);
            }
            Ok((Box::new(server), true))
        }
    }

    pub async fn connect(path: &Path) -> std::io::Result<Stream> {
        Ok(Box::new(ClientOptions::new().open(path)?))
    }

    pub fn remove(_path: &Path) {}
}

pub use imp::{Listener, connect, remove};
