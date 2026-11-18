//! systemd's notify protocol: `READY=1` once the agent runs, and `WATCHDOG=1` from its loop, so
//! that systemd restarts an agent whose loop has stopped (`WatchdogSec=` in
//! `fiber-agent.service`). Without `NOTIFY_SOCKET`, as by hand and in the tests, it does nothing.

use std::ffi::OsStr;
use std::io;
use std::os::unix::net::{SocketAddr, UnixDatagram};

/// The socket systemd listens on, when there is one.
#[derive(Default)]
pub struct Notify {
    socket: Option<(UnixDatagram, SocketAddr)>,
}

impl Notify {
    /// From `NOTIFY_SOCKET`, which systemd sets for a service of `Type=notify`.
    pub fn from_env() -> Notify {
        let Some(path) = std::env::var_os("NOTIFY_SOCKET") else {
            return Notify::default();
        };
        match UnixDatagram::unbound().and_then(|socket| Ok((socket, address(&path)?))) {
            Ok(socket) => Notify { socket: Some(socket) },
            Err(e) => {
                eprintln!("NOTIFY_SOCKET {}: {e}", path.display());
                Notify::default()
            }
        }
    }

    /// Sends one state, as `READY=1`. A message that is lost is not worth stopping for: systemd
    /// only waits longer.
    pub fn send(&self, state: &str) {
        if let Some((socket, address)) = &self.socket {
            let _ = socket.send_to_addr(state.as_bytes(), address);
        }
    }
}

/// A path, or a name in the abstract namespace when it starts with `@`.
#[cfg(target_os = "linux")]
fn address(path: &OsStr) -> io::Result<SocketAddr> {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::ffi::OsStrExt;
    match path.as_bytes().split_first() {
        Some((b'@', name)) => SocketAddr::from_abstract_name(name),
        _ => SocketAddr::from_pathname(path),
    }
}

#[cfg(not(target_os = "linux"))]
fn address(path: &OsStr) -> io::Result<SocketAddr> {
    SocketAddr::from_pathname(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_states_reach_the_socket() {
        let dir = crate::testdir::TestDir::new("notify");
        let path = dir.path().join("notify.sock");
        let listener = UnixDatagram::bind(&path).unwrap();
        let notify = Notify {
            socket: Some((
                UnixDatagram::unbound().unwrap(),
                address(path.as_os_str()).unwrap(),
            )),
        };
        notify.send("READY=1");
        notify.send("WATCHDOG=1");
        let mut buf = [0; 64];
        let n = listener.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"READY=1");
        let n = listener.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"WATCHDOG=1");
    }

    #[test]
    fn without_a_socket_nothing_happens() {
        Notify::default().send("WATCHDOG=1");
    }
}
