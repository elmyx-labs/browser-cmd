//! Copying a tunnel's bytes both ways until it goes idle.

use std::time::Duration;

use crate::{Error, Result};

/// Observe progress in either direction without a new task or timer per byte.
/// The ordinary bidirectional copier retains its half-close behavior.
struct ActiveStream<'a, T> {
    stream: &'a mut T,
    activity: &'a std::sync::Mutex<tokio::time::Instant>,
}

impl<T: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for ActiveStream<'_, T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = std::pin::Pin::new(&mut *this.stream).poll_read(cx, buf);
        if buf.filled().len() > before {
            *this.activity.lock().unwrap() = tokio::time::Instant::now();
            super::CARRIED
                .fetch_add((buf.filled().len() - before) as u64, std::sync::atomic::Ordering::Relaxed);
        }
        result
    }
}

impl<T: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for ActiveStream<'_, T> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let result = std::pin::Pin::new(&mut *this.stream).poll_write(cx, bytes);
        if matches!(result, std::task::Poll::Ready(Ok(n)) if n > 0) {
            *this.activity.lock().unwrap() = tokio::time::Instant::now();
        }
        result
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.get_mut().stream).poll_flush(cx)
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.get_mut().stream).poll_shutdown(cx)
    }
}

pub(super) async fn relay<A, B>(client: &mut A, upstream: &mut B, idle: Duration) -> Result<()>
where
    A: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let activity = std::sync::Mutex::new(tokio::time::Instant::now());
    let mut client = ActiveStream { stream: client, activity: &activity };
    let mut upstream = ActiveStream { stream: upstream, activity: &activity };
    let expired = async {
        loop {
            let last = *activity.lock().unwrap();
            tokio::time::sleep_until(last + idle).await;
            if tokio::time::Instant::now().duration_since(*activity.lock().unwrap()) >= idle {
                break;
            }
        }
    };
    tokio::select! {
        result = tokio::io::copy_bidirectional(&mut client, &mut upstream) => { result?; Ok(()) },
        _ = expired => Err(Error::Timeout("idle proxy tunnel")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test(start_paused = true)]
    async fn active_traffic_survives_beyond_the_idle_budget() {
        let (mut client, mut left) = tokio::io::duplex(1024);
        let (mut right, mut server) = tokio::io::duplex(1024);
        let task =
            tokio::spawn(async move { relay(&mut left, &mut right, Duration::from_millis(200)).await });
        for _ in 0..12 {
            client.write_all(b"x").await.unwrap();
            let mut byte = [0];
            server.read_exact(&mut byte).await.unwrap();
            assert_eq!(&byte, b"x");
            tokio::time::advance(Duration::from_millis(50)).await;
            assert!(!task.is_finished(), "active tunnel expired by absolute age");
        }
        // A half-closed request can still receive its response.
        client.shutdown().await.unwrap();
        assert_eq!(server.read(&mut [0; 1]).await.unwrap(), 0);
        server.write_all(b"response").await.unwrap();
        server.shutdown().await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, b"response");
        task.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn idle_tunnels_expire_without_leaving_readers_waiting() {
        let (mut client, mut left) = tokio::io::duplex(1024);
        let (mut right, _server) = tokio::io::duplex(1024);
        let task =
            tokio::spawn(async move { relay(&mut left, &mut right, Duration::from_millis(200)).await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(201)).await;
        assert!(matches!(task.await.unwrap(), Err(Error::Timeout(_))));
        assert_eq!(client.read(&mut [0; 1]).await.unwrap(), 0);
    }
}
