//! The display (`docs/mqtt-api.md`, `display`): the backlight, a client's message, and the
//! picture of that message. The LCD's driver is in [`super::lcd`].

use std::time::Duration;

use tokio::time::Instant;
use tower_hmi::fonts::SPLEEN_6X12;
use tower_hmi::{Frame, TextStyle};

/// The LCD's picture: 128 x 64 pixels, 16 bytes a row.
pub type Picture = Frame<16, 64>;

/// What a message shows: 5 lines of 21 characters in the 6 x 12 font.
pub const COLUMNS: usize = 21;
pub const ROWS: usize = 5;

/// The backlight dims this long after the last press of a button.
pub const DIM_AFTER: Duration = Duration::from_secs(60);

/// The display as the agent wants it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    /// The backlight's brightness when lit, 0 to 100 %.
    pub backlight: u8,
    /// A client's message, and when it goes.
    pub message: Option<String>,
    pub message_until: Option<Instant>,
    /// The last press of a button: the backlight is lit until [`DIM_AFTER`] after it.
    pub last_press: Instant,
}

impl Default for Display {
    fn default() -> Self {
        Display {
            backlight: 80,
            message: None,
            message_until: None,
            last_press: Instant::now(),
        }
    }
}

impl Display {
    /// The backlight's duty cycle now, 0 to 100 %: its brightness, or a fifth of it once it has
    /// dimmed.
    pub fn duty(&self, now: Instant) -> u8 {
        if now.saturating_duration_since(self.last_press) < DIM_AFTER {
            self.backlight
        } else {
            self.backlight / 5
        }
    }

    /// Takes the message away when its time is up; whether it did.
    pub fn expire(&mut self, now: Instant) -> bool {
        if self.message_until.is_some_and(|until| now >= until) {
            self.message = None;
            self.message_until = None;
            return true;
        }
        false
    }
}

/// A message's lines as they fit: at most [`ROWS`], each cut to [`COLUMNS`] characters.
pub fn lines(text: &str) -> Vec<String> {
    text.split('\n')
        .take(ROWS)
        .map(|line| line.chars().take(COLUMNS).collect())
        .collect()
}

/// The picture of a message: its lines from the top left, one row of the font each.
pub fn draw_message(text: &str) -> Picture {
    let mut picture = Picture::new();
    let style = TextStyle::new(&SPLEEN_6X12);
    let mut canvas = picture.canvas();
    for (row, line) in lines(text).iter().enumerate() {
        canvas.text(0, row as i32 * 12, line, &style);
    }
    picture
}

/// A picture as text, `#` for a lit pixel, one line per row: what the tests compare.
pub fn as_text(picture: &Picture) -> String {
    let mut text = String::new();
    for y in 0..64 {
        for byte in picture.row(y) {
            for bit in (0..8).rev() {
                text.push(if byte >> bit & 1 == 1 { '#' } else { '.' });
            }
        }
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compares a picture with the one stored in `src/agent/pictures/<name>.txt`. With
    /// `FIBER_WRITE_PICTURES=1` it stores it instead (look at it before you commit it).
    fn same_as_stored(name: &str, picture: &Picture) {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("src/agent/pictures/{name}.txt"));
        let text = as_text(picture);
        if std::env::var_os("FIBER_WRITE_PICTURES").is_some() {
            std::fs::write(&path, &text).unwrap();
        }
        let stored = std::fs::read_to_string(&path).unwrap();
        assert!(stored == text, "{name} differs from {}:\n{text}", path.display());
    }

    #[test]
    fn a_message() {
        same_as_stored("message", &draw_message("Back in\n5 minutes"));
    }

    #[test]
    fn a_long_message_is_cut() {
        let text = "123456789012345678901234\n2\n3\n4\n5\n6";
        assert_eq!(lines(text), ["123456789012345678901", "2", "3", "4", "5"]);
        same_as_stored("message-cut", &draw_message(text));
    }

    #[tokio::test(start_paused = true)]
    async fn dimming_and_expiry() {
        let mut display = Display::default();
        let start = Instant::now();
        assert_eq!(display.duty(start), 80);
        assert_eq!(display.duty(start + DIM_AFTER), 16);
        display.last_press = start + DIM_AFTER;
        assert_eq!(display.duty(start + DIM_AFTER), 80);

        display.message = Some("hi".into());
        display.message_until = Some(start + Duration::from_secs(5));
        assert!(!display.expire(start + Duration::from_secs(4)));
        assert!(display.expire(start + Duration::from_secs(5)));
        assert_eq!(display.message, None);
    }
}
