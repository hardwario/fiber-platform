//! The three buttons beside the display (`docs/mqtt-api.md`, `buttons/event`).
//!
//! A thread of their own waits for the lines' edges and feeds `tower-hmi`'s recognizer, which
//! turns presses into clicks, holds and repeats; the agent's loop gets each press as the button
//! goes down (the pages turn on it) and the gestures. The lines are behind [`Lines`], so the
//! tests press made-up buttons.

use std::io;
use std::time::{Duration, Instant};

use tower_hmi::input::{ButtonEvent, Config, Kind, Recognizer};

/// The buttons as bits of a mask, as the recognizer takes them.
pub const UP: u8 = 1 << 0;
pub const ENTER: u8 = 1 << 1;
pub const DOWN: u8 = 1 << 2;

/// How often the lines are read while a button is down.
const SCAN: Duration = Duration::from_millis(10);

/// The longest wait for an edge when nothing is due: the thread looks up once in a while.
const IDLE: Duration = Duration::from_secs(60);

/// The timing: a click within 0.5 s, a hold after 0.8 s, then a repeat every 0.2 s. No double
/// clicks: they would hold every click back.
pub fn config() -> Config {
    Config {
        debounce_ms: 20,
        click_ms: 500,
        hold_ms: 800,
        repeat_ms: Some(200),
        double_click_ms: None,
    }
}

/// The names of the buttons in a mask, as `buttons/event` has them.
pub fn names(mask: u8) -> Vec<&'static str> {
    [(UP, "up"), (ENTER, "enter"), (DOWN, "down")]
        .into_iter()
        .filter(|(bit, _)| mask & bit != 0)
        .map(|(_, name)| name)
        .collect()
}

/// The gestures that go on MQTT; presses and releases do not.
pub fn gesture(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Click => Some("click"),
        Kind::Hold => Some("hold"),
        Kind::Repeat => Some("repeat"),
        _ => None,
    }
}

/// The buttons' lines.
pub trait Lines {
    /// Waits for an edge on any line, at most `timeout`.
    fn wait(&mut self, timeout: Duration) -> io::Result<()>;
    /// The buttons down now, as a mask.
    fn read(&mut self) -> io::Result<u8>;
}

/// Turns what the lines do into gestures, until `gestures` is closed or a line fails.
pub fn watch<L: Lines>(mut lines: L, gestures: tokio::sync::mpsc::Sender<ButtonEvent>) -> io::Result<()> {
    let start = Instant::now();
    let ms = || start.elapsed().as_millis() as u64;
    let mut recognizer = Recognizer::new(config());
    loop {
        let now = ms();
        let mut timeout = recognizer
            .next_deadline()
            .map_or(IDLE, |at| Duration::from_millis(at.saturating_sub(now)));
        if recognizer.needs_polling() && !recognizer.waiting_for_release() {
            timeout = timeout.min(SCAN);
        }
        lines.wait(timeout)?;
        let raw = lines.read()?;
        let mut events = Vec::new();
        recognizer.update(ms(), raw, &mut |event| events.push(event));
        for event in events {
            let wanted = event.kind == Kind::Press || gesture(event.kind).is_some();
            if wanted && gestures.blocking_send(event).is_err() {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds a made-up sequence of (milliseconds, buttons down) to the recognizer, sampled every
    /// 10 ms as the thread does, and gives back the gestures with their time.
    fn gestures(presses: &[(u64, u8)], until: u64) -> Vec<(u64, Vec<&'static str>, &'static str)> {
        let mut recognizer = Recognizer::new(config());
        let mut out = Vec::new();
        for now in (0..=until).step_by(10) {
            let raw = presses
                .iter()
                .rev()
                .find(|(at, _)| *at <= now)
                .map_or(0, |(_, mask)| *mask);
            recognizer.update(now, raw, &mut |event| {
                if let Some(name) = gesture(event.kind) {
                    out.push((now, names(event.buttons), name));
                }
            });
        }
        out
    }

    #[test]
    fn a_click() {
        assert_eq!(
            gestures(&[(100, ENTER), (300, 0)], 1000),
            [(320, vec!["enter"], "click")]
        );
    }

    #[test]
    fn a_hold_and_its_repeats() {
        let seen = gestures(&[(0, DOWN), (1500, 0)], 2000);
        let kinds: Vec<_> = seen.iter().map(|(at, _, kind)| (*at, *kind)).collect();
        // Down after 20 ms of debounce; the hold 0.8 s later; then every 0.2 s.
        assert_eq!(
            kinds,
            [
                (820, "hold"),
                (1020, "repeat"),
                (1220, "repeat"),
                (1420, "repeat")
            ]
        );
        assert!(seen.iter().all(|(_, buttons, _)| buttons == &["down"]));
    }

    #[test]
    fn two_together_and_a_bounce() {
        // A bounce shorter than the debounce is nothing; two pressed together are one click.
        assert!(gestures(&[(100, UP), (110, 0)], 1000).is_empty());
        assert_eq!(
            gestures(&[(100, UP | DOWN), (250, 0)], 1000),
            [(270, vec!["up", "down"], "click")]
        );
    }

    /// Lines that play a script: each wait moves the clock, as the real ones would.
    struct Scripted {
        start: Instant,
        presses: Vec<(u64, u8)>,
    }

    impl Lines for Scripted {
        fn wait(&mut self, timeout: Duration) -> io::Result<()> {
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
            Ok(())
        }

        fn read(&mut self) -> io::Result<u8> {
            let now = self.start.elapsed().as_millis() as u64;
            if now > 600 {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            Ok(self
                .presses
                .iter()
                .rev()
                .find(|(at, _)| *at <= now)
                .map_or(0, |(_, mask)| *mask))
        }
    }

    #[test]
    fn the_thread_sends_the_press_and_the_gesture() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let lines = Scripted {
            start: Instant::now(),
            presses: vec![(50, UP), (200, 0)],
        };
        assert!(watch(lines, tx).is_err());
        // The press as the button goes down (the pages turn on it), then the click on the
        // release; the release itself is not sent.
        let event = rx.try_recv().unwrap();
        assert_eq!((event.buttons, event.kind), (UP, Kind::Press));
        let event = rx.try_recv().unwrap();
        assert_eq!((event.buttons, event.kind), (UP, Kind::Click));
        assert!(rx.try_recv().is_err());
    }
}
