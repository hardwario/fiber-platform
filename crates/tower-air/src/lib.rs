//! fiber-tower-air (`crates/tower-air`): the hardware-free side of the FIBER north bridge's TOWER
//! personality.
//!
//! The north bridge (an STM32WL55) makes a FIBER a gateway for HARDWARIO TOWER radio nodes. TOWER's
//! own gateway runs on another radio chip (SPIRIT1) inside TOWER's firmware SDK, which this
//! repository does not reuse; it reuses TOWER's hardware-free kernels, pinned at one revision
//! (the root `Cargo.toml`), and writes the rest here:
//!
//! * the software packet codec (whitening, length byte, CRC-16), because the STM32WL packet
//!   engine cannot produce SPIRIT1's whitening by itself;
//! * the on-air frame codec (header, AES-128-CCM through `tower-radio`);
//! * the gateway flow (confirmed delivery, replay protection, pairing) over a radio trait, with
//!   every accept or reject decision taken by `tower-net`.
//!
//! The packet codec is here ([`packet`], T23); the frame codec and the gateway flow come with T24.
//! The crate also proves that the pinned TOWER kernels build for the host and for the north
//! bridge's target (`thumbv7em-none-eabi`, `just check`), and holds tests that pin the on-air facts
//! this stack relies on: captures of a real Core Module among them. A test that fails after the
//! TOWER pin moved means TOWER changed something on the air: read `docs/architecture.md` ("Two
//! radio stacks") before touching the test.

#![no_std]

pub mod packet;

#[cfg(test)]
mod tests {
    use tower_net::nonce::{NONCE_LEN, nonce_for};
    use tower_radio::{BITRATE, frame_toa_ms};

    /// The CCM nonce is derived from the clear header, never transmitted:
    /// `src[4] | counter[4] | bulk_index[3] | 0x0000`, little-endian (TOWER, docs/firmware/radio.md,
    /// "Wire protocol"). The north bridge and every TOWER node must build the same 13 bytes.
    #[test]
    fn nonce_layout() {
        assert_eq!(NONCE_LEN, 13);
        assert_eq!(
            nonce_for(0x0403_0201, 0x0807_0605, 0x000B_0A09),
            [
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x00, 0x00
            ]
        );
    }

    /// The duty-cycle budget counts a frame as preamble (4) + sync (4) + length (1) + payload +
    /// CRC (2) at the nominal bit rate, rounded up: 45 ms for the largest frame (96 bytes) and
    /// 17 ms for an ACK (28 bytes), as TOWER's parameter table states.
    #[test]
    fn time_on_air() {
        assert_eq!(BITRATE, 19_200);
        assert_eq!(frame_toa_ms(96), 45);
        assert_eq!(frame_toa_ms(28), 17);
    }
}
