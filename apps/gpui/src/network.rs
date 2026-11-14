//! Whether the computer is on a network: there is a route out. Nothing is sent to find out.

use std::time::Duration;

use gpui_kit::{App, AppContext as _, Entity, Global};

/// Looked at this often.
const EVERY: Duration = Duration::from_secs(5);

pub struct Network {
    pub online: bool,
    /// Set by a test that decided `online` itself: the machine is not looked at again, so a
    /// test on a runner with no route out does not go offline five seconds in.
    pub pinned: bool,
}

impl Network {
    /// Take the computer to be on a network or off it, whatever the machine says, from now on.
    #[cfg(test)]
    pub fn pin(&mut self, online: bool) {
        self.online = online;
        self.pinned = true;
    }
}

#[derive(Clone)]
pub struct NetworkHandle(pub Entity<Network>);

impl Global for NetworkHandle {}

/// The computer is on a network.
pub fn online(cx: &App) -> bool {
    cx.try_global::<NetworkHandle>()
        .is_none_or(|network| network.0.read(cx).online)
}

/// A route out exists: "connecting" a UDP socket picks the route and sends nothing, and fails
/// when there is none (no network, or every interface down).
fn connected() -> bool {
    use std::net::UdpSocket;
    let routed = |local: &str, remote: &str| {
        UdpSocket::bind(local)
            .and_then(|socket| socket.connect(remote))
            .is_ok()
    };
    routed("0.0.0.0:0", "192.0.2.1:9") || routed("[::]:0", "[2001:db8::1]:9")
}

pub fn init(cx: &mut App) {
    let network = cx.new(|_| Network {
        online: connected(),
        pinned: false,
    });
    cx.set_global(NetworkHandle(network.clone()));
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(EVERY).await;
            let now = cx.background_executor().spawn(async { connected() }).await;
            network.update(cx, |network, cx| {
                if !network.pinned && network.online != now {
                    network.online = now;
                    cx.notify();
                }
            });
        }
    })
    .detach();
}
