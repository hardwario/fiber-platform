//! The south bridge, simulated: `fiber-south-core` behind the south link, for the agent's tests
//! (on a pipe, and as `fiber south-simulator` on a pseudo-terminal). Its counts make 12 V on the
//! input and 4.1 V on the battery at a supply of 3.3 V.

use std::io;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, anyhow};
use fiber_protocol::south::{Desired, Hello, Message, Reset, Status};
use fiber_protocol::{FrameDecoder, MAX_WIRE};
use fiber_south_core::Engine;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

/// The CRC-32 its `Hello` names in the tests on a pipe.
pub const IMAGE_CRC: u32 = 0x6f1c_2a90;

/// Answers on `stream` until it ends: a `Hello` to a `HelloRequest`, a `Status` to a `Desired`.
/// While `silent` is set it answers nothing; every `Desired` goes to `seen`, when there is one.
pub async fn simulate<S: AsyncRead + AsyncWrite>(
    stream: S,
    silent: Arc<AtomicBool>,
    seen: Option<mpsc::UnboundedSender<Desired>>,
) -> io::Result<()> {
    simulate_image(stream, IMAGE_CRC, silent, seen).await
}

/// As [`simulate`], with the CRC-32 its `Hello` names.
pub async fn simulate_image<S: AsyncRead + AsyncWrite>(
    stream: S,
    image_crc: u32,
    silent: Arc<AtomicBool>,
    seen: Option<mpsc::UnboundedSender<Desired>>,
) -> io::Result<()> {
    let (mut rd, mut wr) = tokio::io::split(stream);
    let mut decoder = FrameDecoder::<MAX_WIRE>::new();
    let mut engine = Engine::new();
    let mut seq = 0u16;
    let mut buf = [0u8; 64];
    let hello = Hello {
        version: [0, 1, 0],
        image_crc,
        uid: [7; 12],
        reset: Reset::PowerOn,
        fault: None,
        option_bytes: 0xDEFF_E1AA,
        vrefint_cal: 1655,
    };
    loop {
        let n = rd.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        for &byte in &buf[..n] {
            let Some(frame) = decoder.push(byte) else { continue };
            if silent.load(Ordering::Relaxed) {
                continue;
            }
            let answer = match Message::decode(frame) {
                Ok((_, Message::HelloRequest)) => Message::Hello(hello),
                Ok((applied, Message::Desired(desired))) => {
                    engine.desired(desired);
                    if let Some(seen) = &seen {
                        let _ = seen.send(desired);
                    }
                    Message::Status(Status {
                        applied,
                        ports: engine.ports(),
                        vbat: 2544,
                        vin: 2557,
                        vref: 1505,
                        uptime: engine.uptime(),
                        rx_dropped: 0,
                    })
                }
                _ => continue,
            };
            let mut wire = [0u8; MAX_WIRE];
            let n = answer
                .encode(seq, &mut wire)
                .map_err(|e| io::Error::other(format!("{e:?}")))?;
            seq = seq.wrapping_add(1);
            wr.write_all(&wire[..n]).await?;
        }
    }
}

/// `fiber south-simulator <port>`: the simulated south bridge on a serial port, as a test makes it
/// with `socat` (a pair of pseudo-terminals). It says it runs the image this program carries, so
/// that the agent does not try to flash it.
pub fn south_simulator(port: &Path) -> Result<ExitCode> {
    use tokio_serial::SerialPortBuilderExt;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let path = port.to_string_lossy().into_owned();
        let stream = tokio_serial::new(&path, 115_200)
            .open_native_async()
            .map_err(|e| anyhow!("cannot open {path}: {e}"))?;
        let image_crc = super::images::south_crc().unwrap_or(IMAGE_CRC);
        simulate_image(stream, image_crc, Arc::new(AtomicBool::new(false)), None).await?;
        Ok(ExitCode::SUCCESS)
    })
}
