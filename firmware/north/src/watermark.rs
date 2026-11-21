//! The one thing the north bridge keeps (D8): the transmit counter's watermark
//! (`docs/firmware/north.md`), in the last two pages of the flash (memory.x leaves them out of the
//! image).
//!
//! Each value is a record of 8 bytes, the flash's smallest write: the value and its complement,
//! little-endian. A record whose two halves do not match was cut short by a reset and counts for
//! nothing. Records are appended in one page; when it is full the other page is erased and
//! written next, so one of the two always holds the last value. The counter only grows, so the
//! highest record of the two pages is the watermark.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_stm32::flash::{Blocking, Flash};
use fiber_tower_air::gateway::Watermark;

/// A record is being read: a double ECC error now is a record cut short, not a fault.
pub static READING: AtomicBool = AtomicBool::new(false);
/// The NMI handler saw a double ECC error while a record was read (`main.rs`).
pub static ECC_ERROR: AtomicBool = AtomicBool::new(false);

/// The flash's pages, 2 KB on the STM32WL.
const PAGE: u32 = 2048;
/// The two pages, as offsets from the flash's start: 0x0803F000 and 0x0803F800.
const PAGES: [u32; 2] = [0x3_F000, 0x3_F800];
/// One record: the flash's smallest write.
const RECORD: u32 = 8;
const RECORDS: u32 = PAGE / RECORD;

/// What one record holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Record {
    Erased,
    Value(u32),
    /// Written half, or damaged: counts for nothing, and is not written over.
    Torn,
}

pub struct FlashWatermark {
    flash: Flash<'static, Blocking>,
}

impl FlashWatermark {
    pub fn new(flash: Flash<'static, Blocking>) -> Self {
        FlashWatermark { flash }
    }

    fn record(&mut self, page: usize, index: u32) -> Record {
        let mut bytes = [0u8; RECORD as usize];
        // The flash keeps an error-correcting code for each 8 bytes: a word whose write or erase a
        // power cut stopped can read as a double error, which raises the NMI. Its handler clears
        // the flag and says so here, and the record counts as torn.
        ECC_ERROR.store(false, Ordering::SeqCst);
        READING.store(true, Ordering::SeqCst);
        let read = self.flash.blocking_read(PAGES[page] + index * RECORD, &mut bytes);
        READING.store(false, Ordering::SeqCst);
        if read.is_err() || ECC_ERROR.swap(false, Ordering::SeqCst) {
            return Record::Torn;
        }
        let value = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let check = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        match (value, check) {
            (u32::MAX, u32::MAX) => Record::Erased,
            (v, c) if c == !v => Record::Value(v),
            _ => Record::Torn,
        }
    }

    /// The highest value in the two pages and the page it is in; and each page's first erased
    /// record.
    fn scan(&mut self) -> (Option<(u32, usize)>, [Option<u32>; 2]) {
        let mut best: Option<(u32, usize)> = None;
        let mut free = [None; 2];
        for (page, first_free) in free.iter_mut().enumerate() {
            for index in 0..RECORDS {
                match self.record(page, index) {
                    Record::Erased => {
                        *first_free = Some(index);
                        break;
                    }
                    Record::Value(v) if best.is_none_or(|(b, _)| v > b) => best = Some((v, page)),
                    _ => {}
                }
            }
        }
        (best, free)
    }
}

impl Watermark for FlashWatermark {
    fn load(&mut self) -> Option<u32> {
        self.scan().0.map(|(value, _)| value)
    }

    fn store(&mut self, watermark: u32) -> bool {
        let (best, free) = self.scan();
        let page = best.map_or(0, |(_, page)| page);
        let (page, index) = match free[page] {
            Some(index) => (page, index),
            None => {
                // Full: the other page, erased first. This one keeps the last value meanwhile.
                let other = 1 - page;
                if self
                    .flash
                    .blocking_erase(PAGES[other], PAGES[other] + PAGE)
                    .is_err()
                {
                    return false;
                }
                (other, 0)
            }
        };
        let mut bytes = [0u8; RECORD as usize];
        bytes[..4].copy_from_slice(&watermark.to_le_bytes());
        bytes[4..].copy_from_slice(&(!watermark).to_le_bytes());
        if self
            .flash
            .blocking_write(PAGES[page] + index * RECORD, &bytes)
            .is_err()
        {
            return false;
        }
        self.record(page, index) == Record::Value(watermark)
    }
}
