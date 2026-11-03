//! Ending a connection to a loopback HTTP/1.1 server: the sign-in callbacks, and the local
//! fixtures the tests serve.

use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

/// How long a connection whose response is written waits for its client to hang up.
pub const HANG_UP_WAIT: Duration = Duration::from_secs(10);

/// Ends a connection whose response is written in full (`Connection: close`, with its length
/// known): waits for the client to hang up, then closes.
///
/// A server that closes first loses its response on some Windows PCs: there, a network filter
/// that inspects 127.0.0.1 (seen with AdGuard for Windows) turns a FIN that closely follows the
/// response into a reset, and the client reads `os error 10054` instead of what it was sent.
/// A client hangs up as soon as it has read such a response, so this rarely waits.
pub async fn close_after_client(socket: &mut TcpStream) {
    let mut rest = [0u8; 1024];
    let _ = tokio::time::timeout(HANG_UP_WAIT, async {
        while matches!(socket.read(&mut rest).await, Ok(read) if read > 0) {}
    })
    .await;
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::close_after_client;

    /// On a Windows PC whose filter resets the 127.0.0.1 connections a server closes first,
    /// about one response in five was lost (`os error 10054`) while the fixtures shut their
    /// connections right after answering.
    #[tokio::test(flavor = "multi_thread")]
    async fn every_response_reaches_its_client() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n")
                        && matches!(socket.read(&mut byte).await, Ok(1))
                    {
                        head.push(byte[0]);
                    }
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .await;
                    close_after_client(&mut socket).await;
                });
            }
        });
        let client = reqwest::Client::new();
        for _ in 0..200 {
            let response = client.get(&url).send().await.unwrap();
            assert_eq!(response.text().await.unwrap(), "ok");
        }
    }
}
