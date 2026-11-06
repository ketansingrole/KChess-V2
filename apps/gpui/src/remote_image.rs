//! Pictures Lichess serves for the app to show, such as a broadcast's cover. Only Lichess's own image hosts are asked, as Electron's content policy allows
//! (`img-src https://lichess.org https://image.lichess1.org`); each picture is bounded in size,
//! read once and kept for the session. A picture that cannot be had leaves its placeholder.
//! Every picture is read on one background runtime through one client, so the covers of a page
//! share a connection (over HTTP/2, one for all of them) instead of a handshake each.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gpui_kit::*;

/// The hosts pictures may come from.
const HOSTS: [&str; 2] = ["lichess.org", "image.lichess1.org"];
/// A cover is a few hundred kilobytes; anything far larger is not one.
const MAX_BYTES: usize = 4 * 1024 * 1024;

enum State {
    Loading,
    Ready(Arc<Image>),
    Failed,
}

/// The pictures asked for, by address. Views observe it to show a picture once it arrives.
#[derive(Default)]
pub struct RemoteImages {
    images: HashMap<String, State>,
}

struct RemoteImagesHandle(Entity<RemoteImages>);

impl Global for RemoteImagesHandle {}

pub fn handle(cx: &mut App) -> Entity<RemoteImages> {
    if !cx.has_global::<RemoteImagesHandle>() {
        let images = cx.new(|_| RemoteImages::default());
        cx.set_global(RemoteImagesHandle(images));
    }
    cx.global::<RemoteImagesHandle>().0.clone()
}

/// Whether `url` is an https address on one of Lichess's image hosts.
fn allowed(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.port().is_none()
        && parsed.host_str().is_some_and(|host| HOSTS.contains(&host))
}

/// The picture's format from its first bytes; only the formats a cover comes in.
fn format_of(bytes: &[u8]) -> Option<ImageFormat> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [0xff, 0xd8, 0xff, ..] => Some(ImageFormat::Jpeg),
        [b'G', b'I', b'F', b'8', ..] => Some(ImageFormat::Gif),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some(ImageFormat::Webp),
        _ => None,
    }
}

fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(kchess_core::USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        // A redirect could lead off Lichess's hosts.
        .redirect(reqwest::redirect::Policy::none())
}

/// The runtime pictures are read on: one thread for the session, started by the first picture.
/// A client's pooled connections belong to the runtime that opened them, so the client is only
/// used here.
fn runtime() -> Option<&'static tokio::runtime::Handle> {
    static RUNTIME: OnceLock<Option<tokio::runtime::Handle>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            let handle = runtime.handle().clone();
            std::thread::Builder::new()
                .name("kchess-images".into())
                .spawn(move || runtime.block_on(std::future::pending::<()>()))
                .ok()?;
            Some(handle)
        })
        .as_ref()
}

fn client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| client_builder().build().ok())
        .as_ref()
}

async fn fetch(client: &reqwest::Client, url: String) -> Option<Vec<u8>> {
    let mut response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    if response
        .content_length()
        .is_some_and(|length| length as usize > MAX_BYTES)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_BYTES {
            return None;
        }
    }
    Some(bytes)
}

impl RemoteImages {
    /// The picture at `url` once it has arrived; the first ask starts reading it.
    pub fn get(&mut self, url: &str, cx: &mut Context<Self>) -> Option<Arc<Image>> {
        match self.images.get(url) {
            Some(State::Ready(image)) => return Some(image.clone()),
            Some(_) => return None,
            None => {}
        }
        if !allowed(url) {
            self.images.insert(url.to_string(), State::Failed);
            return None;
        }
        self.images.insert(url.to_string(), State::Loading);
        let (done, answer) = futures::channel::oneshot::channel();
        let address = url.to_string();
        match (runtime(), client()) {
            (Some(runtime), Some(client)) => {
                runtime.spawn(async move {
                    let _ = done.send(fetch(client, address).await);
                });
            }
            _ => {
                let _ = done.send(None);
            }
        }
        let url = url.to_string();
        cx.spawn(async move |this, cx| {
            let bytes = answer.await.ok().flatten();
            let _ = this.update(cx, |images, cx| {
                let state = bytes
                    .and_then(|bytes| Some(Image::from_bytes(format_of(&bytes)?, bytes)))
                    .map_or(State::Failed, |image| State::Ready(Arc::new(image)));
                images.images.insert(url, state);
                cx.notify();
            });
        })
        .detach();
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{allowed, client_builder, fetch, format_of, runtime};
    use gpui_kit::ImageFormat;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn only_lichess_image_hosts_over_https() {
        assert!(allowed(
            "https://image.lichess1.org/display?h=400&op=thumbnail&path=relay.jpg"
        ));
        assert!(allowed("https://lichess.org/assets/cover.png"));
        assert!(!allowed("http://image.lichess1.org/cover.jpg"));
        assert!(!allowed("https://image.lichess1.org.example.com/cover.jpg"));
        assert!(!allowed("https://lichess.org:8443/cover.jpg"));
        assert!(!allowed("file:///etc/passwd"));
    }

    #[test]
    fn a_cover_is_known_by_its_first_bytes() {
        assert_eq!(format_of(b"\x89PNG\r\n"), Some(ImageFormat::Png));
        assert_eq!(
            format_of(&[0xff, 0xd8, 0xff, 0xe0]),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(format_of(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageFormat::Webp));
        assert_eq!(format_of(b"<svg xmlns"), None);
    }

    /// A page of covers: each one used to open its own connection, with a TLS handshake each.
    #[test]
    fn the_pictures_of_a_page_share_one_connection() {
        let runtime = runtime().expect("runtime");
        let connections = Arc::new(AtomicUsize::new(0));
        let accepted = Arc::clone(&connections);
        let (bound, address) = std::sync::mpsc::channel();
        runtime.spawn(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            bound.send(listener.local_addr().unwrap()).unwrap();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                accepted.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    // Keep-alive: one answer per request, on the same connection.
                    while let Ok(read) = socket.read(&mut buffer).await {
                        if read == 0 {
                            return;
                        }
                        let body = b"\x89PNG\r\n";
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        );
                        if socket.write_all(head.as_bytes()).await.is_err()
                            || socket.write_all(body).await.is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        let base = format!("http://{}", address.recv().unwrap());
        let client: &'static reqwest::Client =
            Box::leak(Box::new(client_builder().no_proxy().build().unwrap()));
        for cover in 0..5 {
            let url = format!("{base}/cover-{cover}.png");
            let (done, answer) = std::sync::mpsc::channel();
            runtime.spawn(async move {
                let _ = done.send(fetch(client, url).await);
            });
            let bytes = answer.recv().unwrap().expect("picture");
            assert_eq!(format_of(&bytes), Some(ImageFormat::Png));
        }
        assert_eq!(connections.load(Ordering::SeqCst), 1);
    }
}
