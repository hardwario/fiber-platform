//! The FIBER-R2 revision 2.1, as the software sees it: every GPIO line, device path and I2C
//! address. This is the one place in the code for them (AGENTS.md); `docs/hardware.md` is the
//! reference, and the `config.txt` block (`provision/boot/config-fiber.txt`) is checked against
//! this module by a test.
//!
//! What `fiber doctor` uses is used now; the rest is here for the agent (T17).
#![allow(dead_code)]

/// The CM4's GPIO lines (BCM numbers).
pub mod gpio {
    /// `uart3` TXD and RXD: the north bridge's USART1.
    pub const NORTH_UART_TX: u32 = 4;
    pub const NORTH_UART_RX: u32 = 5;
    /// The south bridge's NRST: pulled low or let go, never driven high.
    pub const SOUTH_RESET: u32 = 7;
    /// `uart4` TXD and RXD: the south bridge's USART1.
    pub const SOUTH_UART_TX: u32 = 8;
    pub const SOUTH_UART_RX: u32 = 9;
    /// The mini-PCIe card's DTR (pin 31) and PERST# (pin 22).
    pub const MINIPCIE_DTR: u32 = 10;
    pub const MINIPCIE_PERST: u32 = 11;
    /// The backlight driver's PWM input: PWM channel 1.
    pub const BACKLIGHT_PWM: u32 = 13;
    pub const LCD_RESET: u32 = 16;
    /// The buzzer: low = sound.
    pub const BUZZER: u32 = 17;
    /// The LCD's chip select (SPI6 CE0), through an inverter.
    pub const LCD_CS: u32 = 18;
    /// SPI6 MISO: claimed by the overlay, not routed to the display.
    pub const LCD_MISO: u32 = 19;
    pub const LCD_MOSI: u32 = 20;
    pub const LCD_SCLK: u32 = 21;
    /// The north bridge's NRST: pulled low or let go, never driven high.
    pub const NORTH_RESET: u32 = 22;
    /// The buttons: pressed = low.
    pub const BUTTON_UP: u32 = 23;
    pub const BUTTON_ENTER: u32 = 24;
    pub const BUTTON_DOWN: u32 = 25;
    /// BOOT0 of the south bridge (PA14, also SWCLK) and of the north bridge (PH3).
    pub const SOUTH_BOOT0: u32 = 26;
    pub const NORTH_BOOT0: u32 = 27;

    /// Every line the software uses, with its name.
    pub const ALL: &[(&str, u32)] = &[
        ("NORTH_UART_TX", NORTH_UART_TX),
        ("NORTH_UART_RX", NORTH_UART_RX),
        ("SOUTH_RESET", SOUTH_RESET),
        ("SOUTH_UART_TX", SOUTH_UART_TX),
        ("SOUTH_UART_RX", SOUTH_UART_RX),
        ("MINIPCIE_DTR", MINIPCIE_DTR),
        ("MINIPCIE_PERST", MINIPCIE_PERST),
        ("BACKLIGHT_PWM", BACKLIGHT_PWM),
        ("LCD_RESET", LCD_RESET),
        ("BUZZER", BUZZER),
        ("LCD_CS", LCD_CS),
        ("LCD_MISO", LCD_MISO),
        ("LCD_MOSI", LCD_MOSI),
        ("LCD_SCLK", LCD_SCLK),
        ("NORTH_RESET", NORTH_RESET),
        ("BUTTON_UP", BUTTON_UP),
        ("BUTTON_ENTER", BUTTON_ENTER),
        ("BUTTON_DOWN", BUTTON_DOWN),
        ("SOUTH_BOOT0", SOUTH_BOOT0),
        ("NORTH_BOOT0", NORTH_BOOT0),
    ];
}

/// The board and its revision, as `fiber/status` names it.
pub const HARDWARE: &str = "FIBER-R2 2.1";

/// The GPIO chip of the CM4's own lines (BCM2711).
pub const GPIO_CHIP: &str = "/dev/gpiochip0";

/// The serial link to the north bridge (`uart3`).
pub const NORTH_BRIDGE_UART: &str = "/dev/ttyAMA3";
/// The serial link to the south bridge (`uart4`).
pub const SOUTH_BRIDGE_UART: &str = "/dev/ttyAMA4";
/// The LCD's SPI device (`spi6-1cs`).
pub const LCD_SPI: &str = "/dev/spidev6.0";
/// The clock (PCF85063A, through the `i2c-rtc` overlay).
pub const RTC: &str = "/dev/rtc0";

/// The I2C bus of the board's own chips: the CM4's `i2c_csi_dsi`.
pub const I2C_BUS: u32 = 10;
/// The 1-Wire master, DS2482-800.
pub const DS2482_ADDRESS: u16 = 0x18;
/// The accelerometer, LIS2DH12, and its `WHO_AM_I` register with the value it answers.
pub const LIS2DH12_ADDRESS: u16 = 0x19;
pub const LIS2DH12_WHO_AM_I: (u8, u8) = (0x0f, 0x33);
/// The clock, PCF85063A.
pub const PCF85063A_ADDRESS: u16 = 0x51;

/// The 1-Wire ports, one bus master of the DS2482-800 each.
pub const ONE_WIRE_PORTS: u32 = 8;

/// The 1-Wire devices in sysfs: the DS2482's eight bus masters, and the probes under each
/// (`docs/hardware.md`, "The 1-Wire ports in Linux").
pub const W1_DEVICES: &str = "/sys/bus/w1/devices";
/// The family code of a DS18B20, the start of its name in sysfs.
pub const DS18B20_FAMILY: &str = "28-";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_gpio_is_used_twice() {
        for (i, (name, line)) in gpio::ALL.iter().enumerate() {
            for (other, other_line) in &gpio::ALL[i + 1..] {
                assert_ne!(line, other_line, "{name} and {other} are both GPIO{line}");
            }
        }
    }

    /// The `config.txt` block names GPIO lines too; they must be this module's.
    #[test]
    fn the_config_txt_block_agrees() {
        let block = include_str!("../../provision/boot/config-fiber.txt");
        let lines: Vec<&str> = block
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect();
        use gpio::*;
        let expected = [
            format!("dtoverlay=pwm,pin={BACKLIGHT_PWM},func=4"),
            format!("gpio={BUZZER}=op,dh"),
            format!("gpio={SOUTH_RESET},{NORTH_RESET}=ip,pu"),
            format!("gpio={SOUTH_BOOT0},{NORTH_BOOT0}=ip,pd"),
            format!("gpio={BUTTON_UP}-{BUTTON_DOWN}=ip,pu"),
        ];
        for line in &expected {
            assert!(lines.contains(&line.as_str()), "the block has no `{line}`");
        }
        assert_eq!(BUTTON_ENTER, BUTTON_UP + 1, "gpio=23-25 covers the three buttons");
        assert_eq!(BUTTON_DOWN, BUTTON_UP + 2);
        // No other line of the block names a GPIO line.
        let gpio_lines = lines
            .iter()
            .filter(|l| l.starts_with("gpio=") || l.contains("pin="))
            .count();
        assert_eq!(gpio_lines, expected.len());
        // Neither SPI0 nor the 1-Wire GPIO overlay, nor flow control (the comments name them).
        assert!(
            lines
                .iter()
                .all(|l| !l.contains("dtparam=spi") && !l.contains("w1-gpio") && !l.contains("ctsrts"))
        );
    }
}
