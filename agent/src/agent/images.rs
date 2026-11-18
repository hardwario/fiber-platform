//! The firmware images the program carries (`just build-fiber` builds them in; the program for
//! the tests carries none and never flashes).

use fiber_protocol::crc::crc32;

/// The south bridge's firmware, as a binary image from the start of its flash.
#[cfg(fiber_south_image)]
pub fn south() -> Option<&'static [u8]> {
    Some(include_bytes!(concat!(env!("OUT_DIR"), "/south.bin")))
}

#[cfg(not(fiber_south_image))]
pub fn south() -> Option<&'static [u8]> {
    None
}

/// The CRC-32 of the south bridge's image: what its `Hello` names when it runs it.
pub fn south_crc() -> Option<u32> {
    south().map(crc32)
}
