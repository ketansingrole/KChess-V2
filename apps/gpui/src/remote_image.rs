//! Pictures Lichess serves for the app to show, such as a broadcast's cover (`BroadcastCard.vue`'s
//! `<img>`). Only Lichess's own image hosts are asked, as Electron's content policy allows
//! (`img-src https://lichess.org https://image.lichess1.org`); each picture is bounded in size,
//! read once and kept for the session. A picture that cannot be had leaves its placeholder.

use std::collections::HashMap;
use std::sync::Arc;
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

async fn fetch(url: String) -> Option<Vec<u8>> {
    let client = reqwest::Client::builder()
        .user_agent(kchess_core::USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        // A redirect could lead off Lichess's hosts.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
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
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            let bytes = runtime
                .ok()
                .and_then(|runtime| runtime.block_on(fetch(address)));
            let _ = done.send(bytes);
        });
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
    use super::{allowed, format_of};
    use gpui_kit::ImageFormat;

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
}
