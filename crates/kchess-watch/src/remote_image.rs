//! Pictures Lichess serves for the app to show, such as a broadcast's cover. Only Lichess's own
//! hosts are asked (`lichess.org`, `image.lichess.org` and the older `image.lichess1.org`, which
//! sends to it), and a redirect is followed only between them; each picture is bounded in size,
//! read once and kept for the session. A picture that cannot be had leaves its placeholder, and
//! why is logged. Every picture is read on one background runtime through one client, so the
//! covers of a page share a connection (over HTTP/2, one for all of them) instead of a handshake
//! each. With a local stand-in for Lichess (`TestLichessOrigin`) the same addresses are read there.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gpui_kit::*;
use kchess_app::core::TestLichessOrigin;
use kchess_app::log::ResultExt as _;
use reqwest::Url;

/// The hosts pictures may come from.
const HOSTS: [&str; 3] = ["lichess.org", "image.lichess.org", "image.lichess1.org"];
/// A cover is a few hundred kilobytes; anything far larger is not one.
const MAX_BYTES: usize = 4 * 1024 * 1024;
/// Redirects followed for one picture: the old host sends to the new one, once.
const MAX_REDIRECTS: usize = 3;

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

/// Whether `url` is an https address on one of Lichess's hosts.
fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.host_str().is_some_and(|host| HOSTS.contains(&host))
}

/// Where `url` is read: itself, or its path and query at the stand-in `origin`. A run pointed
/// at Lichess itself (`KCHESS_E2E_LICHESS=https://lichess.org`) reads its pictures where they are.
fn routed(url: &Url, origin: Option<&str>) -> Result<Url, String> {
    let Some(origin) = origin else {
        return Ok(url.clone());
    };
    let mut at = Url::parse(origin).map_err(|error| format!("the stand-in {origin}: {error}"))?;
    if allowed(&at) {
        return Ok(url.clone());
    }
    at.set_path(url.path());
    at.set_query(url.query());
    Ok(at)
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
    kchess_host::http::client()
        .user_agent(kchess_contracts::USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        // `fetch` follows a redirect itself, only to Lichess's hosts.
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

/// The picture at `url` (on Lichess's hosts) and its format, read at the stand-in `origin` when
/// there is one; or why it cannot be had.
async fn fetch(
    client: &reqwest::Client,
    mut url: Url,
    origin: Option<String>,
) -> Result<(ImageFormat, Vec<u8>), String> {
    for _ in 0..=MAX_REDIRECTS {
        let mut response = client
            .get(routed(&url, origin.as_deref())?)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let status = response.status();
        let kind = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|kind| kind.to_str().ok())
            .unwrap_or("no type")
            .to_string();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|location| location.to_str().ok())
                .ok_or_else(|| format!("{status} with nowhere to go"))?;
            let next = url
                .join(location)
                .map_err(|error| format!("{status} to {location}: {error}"))?;
            if !allowed(&next) {
                return Err(format!("{status} to {next}, off Lichess's hosts"));
            }
            url = next;
            continue;
        }
        if !status.is_success() {
            return Err(format!("{status} ({kind})"));
        }
        if response
            .content_length()
            .is_some_and(|length| length as usize > MAX_BYTES)
        {
            return Err(format!("over {MAX_BYTES} bytes"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > MAX_BYTES {
                return Err(format!("over {MAX_BYTES} bytes"));
            }
        }
        let format = format_of(&bytes).ok_or_else(|| format!("not a picture ({kind})"))?;
        return Ok((format, bytes));
    }
    Err(format!("more than {MAX_REDIRECTS} redirects"))
}

impl RemoteImages {
    /// The picture at `url` once it has arrived; the first ask starts reading it.
    pub fn get(&mut self, url: &str, cx: &mut Context<Self>) -> Option<Arc<Image>> {
        match self.images.get(url) {
            Some(State::Ready(image)) => return Some(image.clone()),
            Some(_) => return None,
            None => {}
        }
        let Some(address) = Url::parse(url).ok().filter(allowed) else {
            self.images.insert(url.to_string(), State::Failed);
            return None;
        };
        self.images.insert(url.to_string(), State::Loading);
        let origin = cx
            .try_global::<TestLichessOrigin>()
            .map(|origin| origin.0.clone());
        let (done, answer) = futures::channel::oneshot::channel();
        match (runtime(), client()) {
            (Some(runtime), Some(client)) => {
                runtime.spawn(async move {
                    // The page may have closed; nobody is waiting for the picture then.
                    done.send(fetch(client, address, origin).await).ok();
                });
            }
            _ => {
                done.send(Err("no runtime to read pictures on".into())).ok();
            }
        }
        let url = url.to_string();
        cx.spawn(async move |this, cx| {
            let read = answer
                .await
                .unwrap_or_else(|_| Err("the read was dropped".into()))
                .map_err(|why| format!("The picture {url} could not be had: {why}"));
            this.update(cx, |images, cx| {
                let state = read.warn_on_err().map_or(State::Failed, |(format, bytes)| {
                    State::Ready(Arc::new(Image::from_bytes(format, bytes)))
                });
                images.images.insert(url, state);
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }
}

impl RemoteImages {
    /// How many pictures have arrived, for the end-to-end tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn arrived_count(&self) -> usize {
        self.images
            .values()
            .filter(|state| matches!(state, State::Ready(_)))
            .count()
    }
}

/// What the page's tests read of the pictures.
#[cfg(test)]
pub mod probe {
    use super::{RemoteImages, State};

    /// Whether the picture at `url` has arrived.
    pub(crate) fn arrived(images: &RemoteImages, url: &str) -> bool {
        matches!(images.images.get(url), Some(State::Ready(_)))
    }

    /// Whether the picture at `url` could not be had.
    pub(crate) fn failed(images: &RemoteImages, url: &str) -> bool {
        matches!(images.images.get(url), Some(State::Failed))
    }
}

#[cfg(test)]
mod tests {
    use super::{client_builder, fetch, format_of, runtime};
    use reqwest::Url;

    fn allowed(url: &str) -> bool {
        Url::parse(url).is_ok_and(|url| super::allowed(&url))
    }
    use gpui_kit::ImageFormat;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn only_lichess_image_hosts_over_https() {
        assert!(allowed(
            "https://image.lichess1.org/display?h=400&op=thumbnail&path=relay.jpg"
        ));
        assert!(allowed(
            "https://image.lichess.org/display?fmt=webp&h=400&op=thumbnail&path=relay.webp"
        ));
        assert!(allowed("https://lichess.org/assets/cover.png"));
        assert!(!allowed("http://image.lichess1.org/cover.jpg"));
        assert!(!allowed("https://image.lichess1.org.example.com/cover.jpg"));
        assert!(!allowed("https://lichess.org:8443/cover.jpg"));
        assert!(!allowed("file:///etc/passwd"));
    }

    #[test]
    fn a_stand_in_reads_the_pictures_unless_it_is_lichess() {
        let cover = Url::parse("https://image.lichess.org/display?path=a.webp").unwrap();
        let read = |origin| super::routed(&cover, origin).unwrap().to_string();
        assert_eq!(read(None), cover.as_str());
        assert_eq!(read(Some("https://lichess.org")), cover.as_str());
        assert_eq!(
            read(Some("http://127.0.0.1:9")),
            "http://127.0.0.1:9/display?path=a.webp"
        );
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
            let url = Url::parse(&format!("https://image.lichess.org/cover-{cover}.png")).unwrap();
            let (done, answer) = std::sync::mpsc::channel();
            let origin = Some(base.clone());
            runtime.spawn(async move {
                done.send(fetch(client, url, origin).await).ok();
            });
            let (format, _) = answer.recv().unwrap().expect("picture");
            assert_eq!(format, ImageFormat::Png);
        }
        // A client per picture opened one connection each, every time (5). The pool can still
        // open a second when a request starts before the last answer's connection is back in it
        // (seen under a loaded test run), so reuse is what is checked, not a count of one.
        let opened = connections.load(Ordering::SeqCst);
        assert!(
            opened < 5,
            "every picture opened its own connection ({opened})"
        );
    }
}
