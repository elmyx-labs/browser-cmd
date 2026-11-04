//! One HTTP/1.1 message per connection, Content-Length only, bounded before allocation.

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::{Error, Result};

/// The parts an [`Error::Limit`] can name here, so the status a caller maps
/// them to cannot drift away from the message that raised them.
pub(crate) const HEADER_PART: &str = "HTTP header";
pub(crate) const BODY_PART: &str = "HTTP body";

pub(crate) struct Message {
    pub(crate) start: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    length: usize,
}

impl Message {
    pub(crate) async fn finish<R: AsyncRead + Unpin>(&mut self, stream: &mut R) -> Result<()> {
        let received = self.body.len();
        self.body
            .try_reserve(self.length - received)
            .map_err(|_| Error::Protocol("cannot allocate bounded HTTP body".into()))?;
        self.body.resize(self.length, 0);
        stream.read_exact(&mut self.body[received..]).await?;
        Ok(())
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

pub(crate) async fn read<R: AsyncRead + Unpin>(
    stream: &mut R,
    header_cap: usize,
    body_cap: usize,
) -> Result<Message> {
    let mut message = read_head(stream, header_cap, body_cap).await?;
    message.finish(stream).await?;
    Ok(message)
}

pub(crate) async fn read_head<R: AsyncRead + Unpin>(
    stream: &mut R,
    header_cap: usize,
    body_cap: usize,
) -> Result<Message> {
    let mut bytes = Vec::new();
    let end = loop {
        if let Some(at) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break at + 4;
        }
        if bytes.len() >= header_cap {
            return Err(Error::Limit { part: HEADER_PART, limit: header_cap });
        }
        let mut chunk = [0_u8; 4096];
        let room = chunk.len().min(header_cap - bytes.len());
        let n = stream.read(&mut chunk[..room]).await?;
        if n == 0 {
            return Err(Error::Protocol("EOF in HTTP header".into()));
        }
        bytes.extend_from_slice(&chunk[..n]);
    };
    let header =
        std::str::from_utf8(&bytes[..end]).map_err(|_| Error::Protocol("HTTP header is not UTF-8".into()))?;
    let mut lines = header[..header.len() - 4].split("\r\n");
    let start = lines.next().unwrap_or_default().to_owned();
    if start.is_empty() || start.bytes().any(|b| b < b' ' || b == 127) {
        return Err(Error::Protocol("invalid HTTP start line".into()));
    }
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        let (name, value) =
            line.split_once(':').ok_or_else(|| Error::Protocol("invalid HTTP header line".into()))?;
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || value.bytes().any(|b| (b < b' ' && b != b'\t') || b == 127)
        {
            return Err(Error::Protocol("invalid HTTP header characters".into()));
        }
        let name = name.to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "host" | "origin" | "browser-session" | "authorization"
        ) && headers.iter().any(|(key, _)| key == &name)
        {
            return Err(Error::Protocol(format!("duplicate HTTP {name}")));
        }
        headers.push((name, value.trim().to_owned()));
    }
    if headers.iter().any(|(name, _)| name == "transfer-encoding") {
        return Err(Error::Protocol("Transfer-Encoding is unsupported; use Content-Length".into()));
    }
    let length = match headers.iter().find(|(name, _)| name == "content-length") {
        Some((_, value)) if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
            value.parse::<usize>().map_err(|_| Error::Limit { part: BODY_PART, limit: body_cap })?
        }
        Some(_) => return Err(Error::Protocol("invalid HTTP Content-Length".into())),
        None => 0,
    };
    if length > body_cap {
        return Err(Error::Limit { part: BODY_PART, limit: body_cap });
    }
    let mut body = bytes.split_off(end);
    body.truncate(length);
    Ok(Message { start, headers, body, length })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn reads_exact_body_without_waiting_for_eof() {
        use tokio::io::AsyncWriteExt;
        let (mut writer, mut reader) = tokio::io::duplex(1024);
        writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").await.unwrap();
        let message = tokio::time::timeout(Duration::from_millis(100), read(&mut reader, 1024, 16))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message.body, b"{}");
        assert_eq!(message.header("content-length"), Some("2"));
    }

    #[tokio::test]
    async fn rejects_oversized_or_ambiguous_framing_before_reading_the_body() {
        for raw in [
            "POST /call HTTP/1.1\r\nContent-Length: 999999999999\r\n\r\n",
            "POST /call HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 3\r\n\r\n",
            "POST /call HTTP/1.1\r\nContent-Length: +2\r\n\r\n",
            "POST /call HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n",
            "POST /call HTTP/1.1\r\nHost: localhost\r\nHost: evil.example\r\n\r\n",
            "POST /call HTTP/1.1\r\nHost : localhost\r\n\r\n",
        ] {
            assert!(read(&mut raw.as_bytes(), 1024, 16).await.is_err(), "{raw}");
        }
        assert!(read(&mut b"0123456789".as_slice(), 8, 16).await.is_err());
    }
}
