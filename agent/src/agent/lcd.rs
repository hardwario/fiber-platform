//! The LCD: a Winstar WG12864J3, 128 x 64, with an ST7920-style controller on its serial
//! interface (`docs/hardware.md`). `/dev/spidev6.0` in SPI mode 3 at 400 kHz, chip select on
//! GPIO18 through an inverter, reset on GPIO16.
//!
//! The interface only takes data, nothing is read back. Every byte goes as three: a sync byte
//! (`0xF8` before a command, `0xFA` before data), then the byte's high half and its low half, each
//! in the high four bits. Each such packet is one write of its own, with the chip select around
//! it: the controller needs 72 us for most commands, and a packet at 400 kHz and its system call
//! take about that.
//!
//! The picture goes into the controller's graphics memory, which is 256 x 32: the top half of
//! the screen in its first 128 columns, the bottom half in the next 128. Only the rows that
//! changed are sent. The panel is mounted upside down on the display board, so the picture is
//! turned by 180 degrees on the way, as the earlier software did. Seen on a FIBER (2026-11-19):
//! sent as drawn the pages are upside down, turned they are upright.

use std::io;
use std::time::Duration;

use super::display::Picture;

/// A picture's rows, as the LCD's thread gets them: 64 rows of 16 bytes, the leftmost pixel in
/// the highest bit, a set bit lit.
pub type Rows = [[u8; 16]; 64];

/// The rows of `picture`.
pub fn rows(picture: &Picture) -> Rows {
    std::array::from_fn(|y| *picture.row(y))
}

/// SPI mode 3 (clock idle high, data taken on the rising edge), as the earlier software used.
pub const SPI_MODE: u8 = 3;
/// Slow enough for the controller at 3.3 V.
pub const SPEED_HZ: u32 = 400_000;

/// The sync byte before a command and before data.
const COMMAND: u8 = 0xF8;
const DATA: u8 = 0xFA;

/// The commands used.
const BASIC: u8 = 0x30;
const DISPLAY_ON: u8 = 0x0C;
const CLEAR: u8 = 0x01;
const ENTRY_INCREMENT: u8 = 0x06;
const EXTENDED: u8 = 0x34;
const EXTENDED_GRAPHICS: u8 = 0x36;
/// Sets the vertical, then the horizontal address of the graphics memory.
const ADDRESS: u8 = 0x80;

/// One byte as the controller takes it: the sync byte and the two halves.
pub fn packet(sync: u8, byte: u8) -> [u8; 3] {
    [sync, byte & 0xF0, byte << 4]
}

/// The LCD's bus: the packets, the reset line, and a pause.
pub trait Bus {
    /// Sends one packet, with the chip select around it.
    fn send(&mut self, packet: [u8; 3]) -> io::Result<()>;
    /// Pulls the reset line low, then lets it go high again.
    fn reset(&mut self, low: bool) -> io::Result<()>;
    fn sleep(&mut self, duration: Duration);
}

/// The controller's rows as the panel shows them: row `r` of the controller is row `63 - r` of
/// the picture, its bytes in the other order and the bits of each byte too.
pub fn turned(rows: &Rows, row: usize) -> [u8; 16] {
    let mut bytes = rows[63 - row];
    bytes.reverse();
    bytes.map(u8::reverse_bits)
}

/// The LCD: what it shows now, as far as the agent knows.
pub struct Lcd<B: Bus> {
    bus: B,
    /// The controller's rows as last sent; none until the controller was set up.
    shown: Option<Rows>,
}

impl<B: Bus> Lcd<B> {
    pub fn new(bus: B) -> Self {
        Lcd { bus, shown: None }
    }

    fn command(&mut self, byte: u8, wait: Duration) -> io::Result<()> {
        self.bus.send(packet(COMMAND, byte))?;
        self.bus.sleep(wait);
        Ok(())
    }

    /// Resets the controller and turns its graphics on. Its graphics memory holds whatever it
    /// held before: the next picture is sent whole.
    fn set_up(&mut self) -> io::Result<()> {
        let us = Duration::from_micros;
        let ms = Duration::from_millis;
        self.bus.reset(true)?;
        self.bus.sleep(ms(50));
        self.bus.reset(false)?;
        // The controller wants 40 ms after its reset before the first command.
        self.bus.sleep(ms(100));
        self.command(BASIC, us(200))?;
        self.command(BASIC, us(200))?;
        self.command(DISPLAY_ON, us(200))?;
        // Clears the text memory, which is shown on top of the graphics: 1.6 ms.
        self.command(CLEAR, ms(20))?;
        self.command(ENTRY_INCREMENT, us(200))?;
        self.command(EXTENDED, us(200))?;
        self.command(EXTENDED_GRAPHICS, us(200))?;
        self.shown = Some([[0xA5; 16]; 64]);
        Ok(())
    }

    /// Shows a picture: sets the controller up first if it is not, then sends the rows that
    /// differ from what it shows. After an error the controller is set up again next time.
    pub fn show(&mut self, picture: &Rows) -> io::Result<()> {
        let result = self.send(picture);
        if result.is_err() {
            self.shown = None;
        }
        result
    }

    fn send(&mut self, picture: &Rows) -> io::Result<()> {
        let fresh = self.shown.is_none();
        if fresh {
            self.set_up()?;
        }
        for row in 0..64 {
            let bytes = turned(picture, row);
            // The words (16 pixels, 2 bytes) that changed, first to last; the whole row when the
            // controller's memory is not known. Each costs 2 packets, about 145 us.
            let words = match self.shown {
                Some(shown) if !fresh => changed_words(&shown[row], &bytes),
                _ => Some((0, 7)),
            };
            let Some((first, last)) = words else {
                continue;
            };
            // Row r of the top half is at (r, 0), row r of the bottom half at (r - 32, 8), in
            // words of 16 pixels.
            let (vertical, horizontal) = if row < 32 { (row, 0) } else { (row - 32, 8) };
            self.command(ADDRESS | vertical as u8, Duration::ZERO)?;
            self.command(ADDRESS | (horizontal + first), Duration::ZERO)?;
            for &byte in &bytes[usize::from(first) * 2..usize::from(last) * 2 + 2] {
                self.bus.send(packet(DATA, byte))?;
            }
            if let Some(shown) = self.shown.as_mut() {
                shown[row] = bytes;
            }
        }
        Ok(())
    }
}

/// The first and the last word (2 bytes) in which `new` differs from `old`, or none.
fn changed_words(old: &[u8; 16], new: &[u8; 16]) -> Option<(u8, u8)> {
    let differs = |w: usize| old[2 * w..2 * w + 2] != new[2 * w..2 * w + 2];
    let first = (0..8).find(|&w| differs(w))?;
    let last = (0..8).rev().find(|&w| differs(w))?;
    Some((first as u8, last as u8))
}

/// How long to wait before trying a failed LCD again.
const RETRY: Duration = Duration::from_secs(5);

/// Shows the pictures that come in, skipping to the newest, until `pictures` is closed. Runs on
/// a thread of its own: every packet is a system call. An error is said once, and the picture
/// tried again after [`RETRY`], which sets the LCD up again.
pub fn drive<B: Bus>(mut lcd: Lcd<B>, pictures: std::sync::mpsc::Receiver<Rows>) {
    let mut said = false;
    let Ok(mut picture) = pictures.recv() else {
        return;
    };
    loop {
        if let Some(newer) = pictures.try_iter().last() {
            picture = newer;
        }
        match lcd.show(&picture) {
            Ok(()) => {
                said = false;
                match pictures.recv() {
                    Ok(next) => picture = next,
                    Err(_) => return,
                }
            }
            Err(e) => {
                if !said {
                    eprintln!("the LCD: {e}");
                    said = true;
                }
                std::thread::sleep(RETRY);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_hmi::{Color, Frame};

    /// What the LCD was sent: packets decoded back to commands and data, and the resets.
    #[derive(Default)]
    struct Recorder {
        sent: Vec<(u8, u8)>,
        resets: Vec<bool>,
        fail_after: Option<usize>,
    }

    impl Bus for Recorder {
        fn send(&mut self, packet: [u8; 3]) -> io::Result<()> {
            if self.fail_after.is_some_and(|n| self.sent.len() >= n) {
                return Err(io::Error::other("gone"));
            }
            assert_eq!(packet[1] & 0x0F, 0);
            assert_eq!(packet[2] & 0x0F, 0);
            self.sent.push((packet[0], packet[1] | packet[2] >> 4));
            Ok(())
        }
        fn reset(&mut self, low: bool) -> io::Result<()> {
            self.resets.push(low);
            Ok(())
        }
        fn sleep(&mut self, _duration: Duration) {}
    }

    #[test]
    fn a_byte_goes_as_three() {
        assert_eq!(packet(0xF8, 0x30), [0xF8, 0x30, 0x00]);
        assert_eq!(packet(0xFA, 0xA5), [0xFA, 0xA0, 0x50]);
    }

    #[test]
    fn the_picture_is_turned_by_half_a_turn() {
        let mut picture: Picture = Frame::new();
        // The top left pixel and the one right of it.
        picture.canvas().pixel(0, 0, Color::Black);
        picture.canvas().pixel(1, 0, Color::Black);
        // The controller's last row, its last byte, its two lowest bits: the bottom right.
        assert_eq!(turned(&rows(&picture), 63)[15], 0b0000_0011);
        assert_eq!(turned(&rows(&picture), 0), [0; 16]);
    }

    #[test]
    fn the_first_picture_sets_up_and_goes_whole() {
        let mut lcd = Lcd::new(Recorder::default());
        lcd.show(&[[0; 16]; 64]).unwrap();
        let sent = &lcd.bus.sent;
        assert_eq!(lcd.bus.resets, [true, false]);
        let set_up: Vec<u8> = sent[..7].iter().map(|&(_, b)| b).collect();
        assert_eq!(set_up, [0x30, 0x30, 0x0C, 0x01, 0x06, 0x34, 0x36]);
        assert!(sent[..7].iter().all(|&(sync, _)| sync == COMMAND));
        // 64 rows of two addresses and 16 bytes.
        assert_eq!(sent.len(), 7 + 64 * 18);
        // The controller's row 33 is at vertical 1, horizontal 8.
        let row33 = &sent[7 + 33 * 18..7 + 34 * 18];
        assert_eq!(row33[0], (COMMAND, 0x81));
        assert_eq!(row33[1], (COMMAND, 0x88));
        assert!(row33[2..].iter().all(|&s| s == (DATA, 0)));
    }

    #[test]
    fn only_the_rows_that_changed_are_sent() {
        let mut lcd = Lcd::new(Recorder::default());
        let mut picture: Picture = Frame::new();
        lcd.show(&rows(&picture)).unwrap();
        lcd.bus.sent.clear();
        lcd.show(&rows(&picture)).unwrap();
        assert!(lcd.bus.sent.is_empty());
        // Row 2 of the picture is the controller's row 61: vertical 29 of the bottom half. Only
        // the word that changed goes: two addresses and its two bytes.
        picture.canvas().pixel(127, 2, Color::Black);
        lcd.show(&rows(&picture)).unwrap();
        assert_eq!(
            lcd.bus.sent,
            [(COMMAND, 0x80 | 29), (COMMAND, 0x88), (DATA, 0x80), (DATA, 0x00)]
        );
        // A pixel in the middle of the row, column 64: turned, column 63, the lowest bit of the
        // fourth word.
        lcd.bus.sent.clear();
        picture.canvas().pixel(64, 2, Color::Black);
        lcd.show(&rows(&picture)).unwrap();
        assert_eq!(
            lcd.bus.sent,
            [
                (COMMAND, 0x80 | 29),
                (COMMAND, 0x88 + 3),
                (DATA, 0x00),
                (DATA, 0x01)
            ]
        );
        assert_eq!(changed_words(&[0; 16], &[0; 16]), None);
        let mut row = [0; 16];
        row[3] = 1;
        row[12] = 1;
        assert_eq!(changed_words(&[0; 16], &row), Some((1, 6)));
    }

    #[test]
    fn after_an_error_it_is_set_up_again() {
        let mut lcd = Lcd::new(Recorder {
            fail_after: Some(20),
            ..Recorder::default()
        });
        assert!(lcd.show(&[[0; 16]; 64]).is_err());
        lcd.bus.fail_after = None;
        lcd.bus.sent.clear();
        lcd.show(&[[0; 16]; 64]).unwrap();
        assert_eq!(lcd.bus.resets, [true, false, true, false]);
        assert_eq!(lcd.bus.sent.len(), 7 + 64 * 18);
    }
}
