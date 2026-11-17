//! The south link over any byte stream: the serial port on a FIBER, a pipe in the tests.

use std::io;

use fiber_protocol::south::{Hello, Message, Status};
use fiber_protocol::{Error, FrameDecoder, MAX_WIRE};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// What came from the south bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Hello(Hello),
    Status {
        applied: u16,
        status: Status,
    },
    /// A whole frame with another schema tag: a firmware from another build.
    OtherSchema,
    /// A frame that did not check out, or a message the agent does not take.
    Dropped,
}

/// Reads frames as they come.
pub struct Reader<R> {
    inner: R,
    decoder: FrameDecoder<MAX_WIRE>,
    buf: [u8; 64],
    pos: usize,
    len: usize,
}

impl<R: AsyncRead + Unpin> Reader<R> {
    pub fn new(inner: R) -> Self {
        Reader {
            inner,
            decoder: FrameDecoder::new(),
            buf: [0; 64],
            pos: 0,
            len: 0,
        }
    }

    /// The next frame's event. Fails when the stream ends or breaks.
    pub async fn next(&mut self) -> io::Result<Event> {
        loop {
            if self.pos == self.len {
                self.len = self.inner.read(&mut self.buf).await?;
                self.pos = 0;
                if self.len == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
            }
            while self.pos < self.len {
                let byte = self.buf[self.pos];
                self.pos += 1;
                if let Some(frame) = self.decoder.push(byte) {
                    return Ok(match Message::decode(frame) {
                        Ok((_, Message::Hello(hello))) => Event::Hello(hello),
                        Ok((_, Message::Status(status))) => Event::Status {
                            applied: status.applied,
                            status,
                        },
                        Err(Error::SchemaMismatch { .. }) => Event::OtherSchema,
                        _ => Event::Dropped,
                    });
                }
            }
        }
    }
}

/// Writes frames, each with the next `seq`.
pub struct Writer<W> {
    inner: W,
    seq: u16,
}

impl<W: AsyncWrite + Unpin> Writer<W> {
    pub fn new(inner: W) -> Self {
        Writer { inner, seq: 0 }
    }

    /// Sends one message; returns its `seq`.
    pub async fn send(&mut self, message: &Message) -> io::Result<u16> {
        let mut wire = [0u8; MAX_WIRE];
        let n = message
            .encode(self.seq, &mut wire)
            .map_err(|e| io::Error::other(format!("{e:?}")))?;
        self.inner.write_all(&wire[..n]).await?;
        self.inner.flush().await?;
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        Ok(seq)
    }
}
