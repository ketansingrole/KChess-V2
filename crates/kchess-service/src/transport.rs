//! The local connection between clients and the service: a Unix domain socket on macOS and
//! Linux, a named pipe on Windows. Either admits this user alone: a Unix connection's peer is
//! checked, and the pipe's DACL grants this user only, while a client refuses a pipe another user
//! owns (`docs/shared-core.md`, "Who may connect"); the token in `hello` covers both.

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
        /// Who may open the pipe: this user alone.
        security: security::OwnerOnly,
        /// The pipe instance waiting for the next client.
        next: Mutex<Option<NamedPipeServer>>,
    }

    impl Listener {
        pub fn bind(path: &Path) -> std::io::Result<Listener> {
            let name = path.to_string_lossy().to_string();
            let security = security::OwnerOnly::new()?;
            let mut first = ServerOptions::new();
            first.first_pipe_instance(true).reject_remote_clients(true);
            let first = security.create(&first, &name)?;
            Ok(Listener {
                name,
                security,
                next: Mutex::new(Some(first)),
            })
        }

        fn instance(&self) -> std::io::Result<NamedPipeServer> {
            let mut options = ServerOptions::new();
            options.reject_remote_clients(true);
            self.security.create(&options, &self.name)
        }

        /// The next connection. Every instance of the pipe admits this user alone (its DACL), so
        /// no other account on the machine can open it; the token in `hello` is the check after.
        pub async fn accept(&self) -> std::io::Result<(Stream, bool)> {
            let server = self
                .next
                .lock()
                .map_err(|_| std::io::Error::other("the pipe is unavailable"))?
                .take();
            let server = match server {
                Some(server) => server,
                None => self.instance()?,
            };
            server.connect().await?;
            let following = self.instance()?;
            if let Ok(mut next) = self.next.lock() {
                *next = Some(following);
            }
            Ok((Box::new(server), true))
        }
    }

    /// Opens the pipe, refusing one this user does not own: pipe names are one namespace for the
    /// whole machine, so another account could create the name first and pose as the core.
    pub async fn connect(path: &Path) -> std::io::Result<Stream> {
        let client = ClientOptions::new().open(path)?;
        if !security::owned_by_this_user(&client)? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the KChess core's pipe belongs to another user",
            ));
        }
        Ok(Box::new(client))
    }

    pub fn remove(_path: &Path) {}

    /// The Win32 security calls the pipe needs, behind safe functions.
    #[allow(unsafe_code)]
    mod security {
        use std::os::windows::io::AsRawHandle;
        use std::ptr::null_mut;

        use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
        use windows_sys::Win32::Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SDDL_REVISION_1, SE_KERNEL_OBJECT,
        };
        use windows_sys::Win32::Security::{
            EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
            SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        /// This process's user, as its token reports it.
        struct User {
            /// A `TOKEN_USER` and the SID it points into, aligned for it.
            buffer: Vec<u64>,
        }

        impl User {
            fn current() -> std::io::Result<User> {
                let mut token: HANDLE = null_mut();
                // SAFETY: `GetCurrentProcess` returns a pseudo-handle needing no close; `token`
                // receives a handle closed below.
                if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let mut length = 0u32;
                // SAFETY: a null buffer of length 0 asks only for the size needed.
                unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut length) };
                let mut buffer = vec![0u64; (length as usize).div_ceil(8).max(1)];
                // SAFETY: `buffer` holds at least `length` bytes, aligned for `TOKEN_USER`.
                let read = unsafe {
                    GetTokenInformation(
                        token,
                        TokenUser,
                        buffer.as_mut_ptr().cast(),
                        length,
                        &mut length,
                    )
                };
                // SAFETY: `token` is the handle opened above, closed once.
                unsafe { CloseHandle(token) };
                if read == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(User { buffer })
            }

            fn sid(&self) -> PSID {
                // SAFETY: `buffer` holds the `TOKEN_USER` written by `GetTokenInformation`.
                unsafe { (*self.buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid }
            }

            /// The SID as text (`S-1-5-21-…`), for a security descriptor.
            fn sid_text(&self) -> std::io::Result<String> {
                let mut text: *mut u16 = null_mut();
                // SAFETY: the SID is valid while `self` lives; `text` receives a string freed
                // with `LocalFree` below.
                if unsafe { ConvertSidToStringSidW(self.sid(), &mut text) } == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let mut length = 0;
                // SAFETY: `text` is NUL-terminated.
                while unsafe { *text.add(length) } != 0 {
                    length += 1;
                }
                // SAFETY: `length` units precede the NUL.
                let sid =
                    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) });
                // SAFETY: allocated by `ConvertSidToStringSidW`, freed once.
                unsafe { LocalFree(text.cast()) };
                Ok(sid)
            }
        }

        /// A security descriptor owned by this user that grants this user alone full access.
        pub struct OwnerOnly {
            descriptor: PSECURITY_DESCRIPTOR,
        }

        // SAFETY: the descriptor is immutable once made and only read by `create`.
        unsafe impl Send for OwnerOnly {}
        // SAFETY: as above.
        unsafe impl Sync for OwnerOnly {}

        impl OwnerOnly {
            pub fn new() -> std::io::Result<OwnerOnly> {
                let sid = User::current()?.sid_text()?;
                // Owner this user; a protected DACL with one entry: this user, all access.
                let sddl: Vec<u16> = format!("O:{sid}D:P(A;;GA;;;{sid})")
                    .encode_utf16()
                    .chain([0])
                    .collect();
                let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
                // SAFETY: `sddl` is NUL-terminated; `descriptor` receives memory freed in `drop`.
                let made = unsafe {
                    ConvertStringSecurityDescriptorToSecurityDescriptorW(
                        sddl.as_ptr(),
                        SDDL_REVISION_1,
                        &mut descriptor,
                        null_mut(),
                    )
                };
                if made == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(OwnerOnly { descriptor })
            }

            /// A pipe instance of `options` at `name` with this descriptor.
            pub fn create(
                &self,
                options: &ServerOptions,
                name: &str,
            ) -> std::io::Result<NamedPipeServer> {
                let mut attributes = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: self.descriptor,
                    bInheritHandle: 0,
                };
                // SAFETY: `attributes` and the descriptor it points to outlive the call, which
                // copies them into the new pipe.
                unsafe {
                    options.create_with_security_attributes_raw(name, (&raw mut attributes).cast())
                }
            }
        }

        impl Drop for OwnerOnly {
            fn drop(&mut self) {
                // SAFETY: allocated by `ConvertStringSecurityDescriptorToSecurityDescriptorW`,
                // freed once.
                unsafe { LocalFree(self.descriptor) };
            }
        }

        /// Whether `pipe` is owned by this process's user.
        pub fn owned_by_this_user(pipe: &impl AsRawHandle) -> std::io::Result<bool> {
            let user = User::current()?;
            let (mut owner, mut descriptor): (PSID, PSECURITY_DESCRIPTOR) =
                (null_mut(), null_mut());
            // SAFETY: the handle is open for the call (a client handle has READ_CONTROL);
            // `owner` points into `descriptor`, freed below.
            let status = unsafe {
                GetSecurityInfo(
                    pipe.as_raw_handle(),
                    SE_KERNEL_OBJECT,
                    OWNER_SECURITY_INFORMATION,
                    &mut owner,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    &mut descriptor,
                )
            };
            if status != 0 {
                return Err(std::io::Error::from_raw_os_error(status as i32));
            }
            // SAFETY: both SIDs are valid: `owner` while `descriptor` lives, the user's while
            // `user` does.
            let same = unsafe { EqualSid(owner, user.sid()) } != 0;
            // SAFETY: allocated by `GetSecurityInfo`, freed once.
            unsafe { LocalFree(descriptor) };
            Ok(same)
        }
    }
}

pub use imp::{Listener, connect, remove};
