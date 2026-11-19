//! The front panel's icons: small bitmaps, one byte a row, the most significant bit on the left,
//! a set bit drawn (`tower_hmi::Image`). Written as binary literals, so each can be read here.

use tower_hmi::Image;

/// The FIBER: a box with its antenna. The first page.
pub static FIBER: Image = Image::new(
    8,
    8,
    &[
        0b0000_0010,
        0b0000_0100,
        0b0000_1000,
        0b1111_1111,
        0b1000_0001,
        0b1010_1001,
        0b1000_0001,
        0b1111_1111,
    ],
);

/// A list with a mark on each line: the components.
pub static COMPONENTS: Image = Image::new(
    8,
    8,
    &[
        0b1100_0000,
        0b1101_1111,
        0b0000_0000,
        0b1100_0000,
        0b1101_1111,
        0b0000_0000,
        0b1100_0000,
        0b1101_1111,
    ],
);

/// A lightning bolt: the power.
pub static POWER: Image = Image::new(
    8,
    8,
    &[
        0b0000_1100,
        0b0001_1000,
        0b0011_0000,
        0b0111_1110,
        0b0000_1100,
        0b0001_1000,
        0b0011_0000,
        0b0100_0000,
    ],
);

/// A thermometer: a thin stem with its marks, and the bulb.
pub static THERMOMETER: Image = Image::new(
    8,
    8,
    &[
        0b0010_0000,
        0b0101_0110,
        0b0101_0000,
        0b0111_0110,
        0b0111_0000,
        0b1111_1000,
        0b1111_1000,
        0b0111_0000,
    ],
);

/// A gear: an apply runs.
pub static GEAR: Image = Image::new(
    8,
    8,
    &[
        0b0001_1000,
        0b0101_1010,
        0b0010_0100,
        0b1100_0011,
        0b1100_0011,
        0b0010_0100,
        0b0101_1010,
        0b0001_1000,
    ],
);

/// An "i" in a ring: about.
pub static INFO: Image = Image::new(
    8,
    8,
    &[
        0b0011_1100,
        0b0110_0110,
        0b1111_1111,
        0b1110_0111,
        0b1110_0111,
        0b1110_0111,
        0b0110_0110,
        0b0011_1100,
    ],
);

/// A ring with a gap and an arrow: a restart.
pub static RESTART: Image = Image::new(
    8,
    8,
    &[
        0b0011_0111,
        0b0100_0011,
        0b1000_0101,
        0b1000_0000,
        0b1000_0001,
        0b1000_0001,
        0b0100_0010,
        0b0011_1100,
    ],
);

/// A globe: an address on the network.
pub static NETWORK: Image = Image::new(
    8,
    8,
    &[
        0b0011_1100,
        0b0101_1010,
        0b1001_1001,
        0b1111_1111,
        0b1001_1001,
        0b0101_1010,
        0b0011_1100,
        0b0000_0000,
    ],
);

/// A tag: the FIBER's name.
pub static NAME: Image = Image::new(
    8,
    8,
    &[
        0b1111_1000,
        0b1000_0100,
        0b1011_0010,
        0b1011_0001,
        0b1000_0001,
        0b1000_0010,
        0b1000_0100,
        0b1111_1000,
    ],
);

/// A plug: PoE.
pub static PLUG: Image = Image::new(
    8,
    8,
    &[
        0b0010_0100,
        0b0010_0100,
        0b1111_1111,
        0b1111_1111,
        0b0111_1110,
        0b0011_1100,
        0b0001_1000,
        0b0001_1000,
    ],
);

/// A small battery: on the battery.
pub static BATTERY: Image = Image::new(
    8,
    8,
    &[
        0b0001_1000,
        0b0111_1110,
        0b0100_0010,
        0b0101_1010,
        0b0101_1010,
        0b0101_1010,
        0b0100_0010,
        0b0111_1110,
    ],
);

/// A question mark in a box: not known.
pub static UNKNOWN: Image = Image::new(
    8,
    8,
    &[
        0b0011_1100,
        0b0110_0110,
        0b0000_0110,
        0b0000_1100,
        0b0001_1000,
        0b0001_1000,
        0b0000_0000,
        0b0001_1000,
    ],
);

/// A check mark: a component that is up.
pub static OK: Image = Image::new(
    7,
    7,
    &[
        0b0000_0010,
        0b0000_0110,
        0b1000_1100,
        0b1101_1000,
        0b0111_0000,
        0b0010_0000,
        0b0000_0000,
    ],
);

/// A cross: a component that failed.
pub static FAILED: Image = Image::new(
    7,
    7,
    &[
        0b1100_0110,
        0b0110_1100,
        0b0011_1000,
        0b0001_0000,
        0b0011_1000,
        0b0110_1100,
        0b1100_0110,
    ],
);

/// A ring: a component in any other state.
pub static OTHER: Image = Image::new(
    7,
    7,
    &[
        0b0011_1000,
        0b0100_0100,
        0b1000_0010,
        0b1000_0010,
        0b1000_0010,
        0b0100_0100,
        0b0011_1000,
    ],
);
