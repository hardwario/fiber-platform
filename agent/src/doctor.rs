//! `fiber doctor`: is the board's hardware there as Linux should see it?
//!
//! It reads `config.txt` for lines that break this board, and looks for what the `base`
//! component brings: the two serial links, the LCD's SPI device, the clock, the chips on the I2C
//! bus with their drivers, and the eight 1-Wire bus masters. Each finding says what to do. It
//! talks to no microcontroller and needs no agent.

use std::process::{Command, ExitCode};

use anyhow::Result;

use crate::board;
use crate::paths::Paths;

/// One thing looked at.
#[derive(Debug, PartialEq)]
pub struct Finding {
    pub ok: bool,
    /// What was looked at, or what is wrong.
    pub what: String,
    /// What to do about it, when it is not ok.
    pub todo: Option<String>,
}

fn ok(what: impl Into<String>) -> Finding {
    Finding {
        ok: true,
        what: what.into(),
        todo: None,
    }
}

fn wrong(what: impl Into<String>, todo: impl Into<String>) -> Finding {
    Finding {
        ok: false,
        what: what.into(),
        todo: Some(todo.into()),
    }
}

/// The lines of `config.txt` that break this board, and whether the platform's block is there.
pub fn config_txt(text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let n = index + 1;
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        let problem = if compact == "dtparam=spi=on" || compact == "dtparam=spi" {
            Some(
                "turns on SPI0, which takes GPIO7 to 11: the south bridge's reset and serial link, and the card's lines",
            )
        } else if compact.starts_with("dtoverlay=w1-gpio") {
            Some(
                "puts 1-Wire on GPIO4, the north bridge's serial link; the board's 1-Wire is the DS2482 on I2C",
            )
        } else if compact.starts_with("dtoverlay=") && compact.contains("ctsrts") {
            Some("gives a UART flow control lines, which takes GPIO lines the board uses for other things")
        } else if compact == "dtoverlay=pwm"
            || (compact.starts_with("dtoverlay=pwm,") && !compact.contains("pin="))
        {
            Some(
                "puts PWM on GPIO18, the LCD's chip select; the backlight's line is `dtoverlay=pwm,pin=13,func=4`",
            )
        } else {
            None
        };
        if let Some(why) = problem {
            findings.push(wrong(
                format!("config.txt, line {n}: `{line}` {why}"),
                format!("remove line {n} from config.txt, then restart"),
            ));
        }
    }
    if text.contains("# BEGIN FIBER PLATFORM") && text.contains("# END FIBER PLATFORM") {
        findings.push(ok("config.txt has the FIBER block"));
    } else {
        findings.push(wrong(
            "config.txt has no FIBER block",
            "turn on the base component: sudo fiber platform enable base, then sudo fiber platform apply",
        ));
    }
    findings
}

/// A device file that the block makes.
fn device(paths: &Paths, path: &str, what: &str) -> Finding {
    if paths.under(path).exists() {
        ok(format!("{path} ({what})"))
    } else {
        wrong(
            format!("{path} ({what}) is missing"),
            "check that config.txt has the FIBER block and restart; `fiber doctor` lists any line in it that gets in the way",
        )
    }
}

/// An I2C chip that a driver has taken.
fn driver(paths: &Paths, address: u16, what: &str) -> Finding {
    let dir = paths.under(&format!(
        "/sys/bus/i2c/devices/{}-{address:04x}/driver",
        board::I2C_BUS
    ));
    if dir.exists() {
        ok(format!(
            "{what} at 0x{address:02x} on I2C bus {} has its driver",
            board::I2C_BUS
        ))
    } else {
        wrong(
            format!(
                "{what} at 0x{address:02x} on I2C bus {} has no driver",
                board::I2C_BUS
            ),
            "check that config.txt has the FIBER block and restart; if it stays, see docs/bench-checks.md (the I2C bus)",
        )
    }
}

/// The accelerometer answers its `WHO_AM_I` register (with `i2cget`, from i2c-tools).
fn accelerometer(paths: &Paths) -> Finding {
    let (register, value) = board::LIS2DH12_WHO_AM_I;
    let what = format!(
        "the LIS2DH12 accelerometer at 0x{:02x} on I2C bus {}",
        board::LIS2DH12_ADDRESS,
        board::I2C_BUS
    );
    if !paths.is_real() {
        return ok(format!("{what}: not asked (a test root)"));
    }
    let answer = Command::new("i2cget")
        .args(["-y", &board::I2C_BUS.to_string()])
        .arg(format!("0x{:02x}", board::LIS2DH12_ADDRESS))
        .arg(format!("0x{register:02x}"))
        .output();
    match answer {
        Err(_) => wrong(
            format!("{what}: not asked, i2cget is missing"),
            "sudo apt install i2c-tools",
        ),
        Ok(out) if String::from_utf8_lossy(&out.stdout).trim() == format!("0x{value:02x}") => {
            ok(format!("{what} answers"))
        }
        Ok(out) => wrong(
            format!(
                "{what} does not answer ({})",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            "run it as root; if it stays, see docs/bench-checks.md (the I2C bus)",
        ),
    }
}

/// The 1-Wire bus masters, one for each port, and no other: the agent takes them in the order of
/// their numbers (`docs/hardware.md`, "The 1-Wire ports in Linux").
fn one_wire(paths: &Paths) -> Finding {
    let found = crate::agent::thermometers::bus_masters(&paths.under(board::W1_DEVICES)).len();
    let ports = board::ONE_WIRE_PORTS as usize;
    if found == ports {
        ok(format!("{found} 1-Wire bus masters, one for each port"))
    } else if found > ports {
        wrong(
            format!("{found} 1-Wire bus masters, and the board has {ports}"),
            "another 1-Wire master (a w1-gpio overlay, a DS2490 on USB) mixes up the ports; take it away, and restart",
        )
    } else {
        wrong(
            format!("{found} of {ports} 1-Wire bus masters"),
            "check that config.txt has the FIBER block and the fiber-ds2482 overlay is in /boot/firmware/overlays, and restart",
        )
    }
}

/// Everything `fiber doctor` looks at, in order.
pub fn check(paths: &Paths) -> Vec<Finding> {
    let config = paths.boot_dir().join("config.txt");
    let mut findings = match std::fs::read_to_string(&config) {
        Ok(text) => config_txt(&text),
        Err(e) => vec![wrong(
            format!("cannot read {}: {e}", config.display()),
            "is this a FIBER?",
        )],
    };
    findings.push(device(
        paths,
        board::NORTH_BRIDGE_UART,
        "the north bridge's serial link",
    ));
    findings.push(device(
        paths,
        board::SOUTH_BRIDGE_UART,
        "the south bridge's serial link",
    ));
    findings.push(device(paths, board::LCD_SPI, "the LCD"));
    findings.push(device(paths, board::RTC, "the clock"));
    findings.push(driver(paths, board::DS2482_ADDRESS, "the DS2482 1-Wire master"));
    findings.push(driver(paths, board::PCF85063A_ADDRESS, "the PCF85063A clock"));
    findings.push(accelerometer(paths));
    findings.push(one_wire(paths));
    findings
}

/// The findings as `fiber doctor` prints them.
pub fn text(findings: &[Finding]) -> String {
    let mut out = String::new();
    for finding in findings {
        out.push_str(&format!(
            "{}  {}\n",
            if finding.ok { "ok  " } else { "FAIL" },
            finding.what
        ));
        if let Some(todo) = &finding.todo {
            out.push_str(&format!("      {todo}\n"));
        }
    }
    let failed = findings.iter().filter(|f| !f.ok).count();
    if failed == 0 {
        out.push_str("Everything is as it should be.\n");
    } else {
        out.push_str(&format!("{failed} of {} not as it should be.\n", findings.len()));
    }
    out
}

/// `fiber doctor`.
pub fn doctor(paths: &Paths) -> Result<ExitCode> {
    let findings = check(paths);
    print!("{}", text(&findings));
    Ok(if findings.iter().all(|f| f.ok) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// For the tests: a root with what a working FIBER has.
#[cfg(test)]
pub fn make_working_root(root: &std::path::Path) {
    let block = format!(
        "arm_64bit=1\n# BEGIN FIBER PLATFORM\n{}\n# END FIBER PLATFORM\n",
        include_str!("../../provision/boot/config-fiber.txt")
    );
    let write = |path: &str, text: &str| {
        let path = root.join(path.trim_start_matches('/'));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write("/boot/firmware/config.txt", &block);
    for device in [
        board::NORTH_BRIDGE_UART,
        board::SOUTH_BRIDGE_UART,
        board::LCD_SPI,
        board::RTC,
    ] {
        write(device, "");
    }
    for address in [board::DS2482_ADDRESS, board::PCF85063A_ADDRESS] {
        std::fs::create_dir_all(root.join(format!(
            "sys/bus/i2c/devices/{}-{address:04x}/driver",
            board::I2C_BUS
        )))
        .unwrap();
    }
    for n in 1..=board::ONE_WIRE_PORTS {
        std::fs::create_dir_all(root.join(format!("sys/bus/w1/devices/w1_bus_master{n}"))).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    #[test]
    fn lines_that_break_the_board() {
        let text = "dtparam=spi=on\n#dtparam=spi=on\ndtoverlay=w1-gpio,gpiopin=4\ndtoverlay=uart3,ctsrts\n\
                    dtoverlay=pwm\ndtoverlay=pwm,pin=13,func=4\ndtoverlay=uart4\n";
        let findings = config_txt(text);
        let wrong: Vec<&str> = findings
            .iter()
            .filter(|f| !f.ok)
            .map(|f| f.what.as_str())
            .collect();
        assert_eq!(wrong.len(), 5, "{wrong:#?}");
        assert!(wrong[0].starts_with("config.txt, line 1: `dtparam=spi=on` turns on SPI0"));
        assert!(wrong[1].starts_with("config.txt, line 3: `dtoverlay=w1-gpio,gpiopin=4`"));
        assert!(wrong[2].starts_with("config.txt, line 4: `dtoverlay=uart3,ctsrts`"));
        assert!(wrong[3].starts_with("config.txt, line 5: `dtoverlay=pwm` puts PWM on GPIO18"));
        assert_eq!(wrong[4], "config.txt has no FIBER block");
        assert_eq!(
            findings[0].todo.as_deref(),
            Some("remove line 1 from config.txt, then restart")
        );
    }

    #[test]
    fn a_working_fiber() {
        let tmp = TestDir::new("doctor");
        make_working_root(tmp.path());
        let findings = check(&Paths::new(Some(tmp.path().to_path_buf())));
        assert!(findings.iter().all(|f| f.ok), "{}", text(&findings));
        assert!(text(&findings).ends_with("Everything is as it should be.\n"));
    }

    #[test]
    fn a_fiber_without_its_devices() {
        let tmp = TestDir::new("doctor");
        make_working_root(tmp.path());
        std::fs::remove_file(tmp.path().join("dev/ttyAMA4")).unwrap();
        std::fs::remove_dir_all(tmp.path().join("sys/bus/w1/devices/w1_bus_master8")).unwrap();
        let findings = check(&Paths::new(Some(tmp.path().to_path_buf())));
        let out = text(&findings);
        assert!(
            out.contains("FAIL  /dev/ttyAMA4 (the south bridge's serial link) is missing\n"),
            "{out}"
        );
        assert!(out.contains("FAIL  7 of 8 1-Wire bus masters\n"), "{out}");
        assert!(out.ends_with("2 of 9 not as it should be.\n"), "{out}");

        // Nine: another master would shift the ports.
        for n in [8, 9] {
            std::fs::create_dir_all(tmp.path().join(format!("sys/bus/w1/devices/w1_bus_master{n}"))).unwrap();
        }
        let out = text(&check(&Paths::new(Some(tmp.path().to_path_buf()))));
        assert!(
            out.contains("FAIL  9 1-Wire bus masters, and the board has 8\n"),
            "{out}"
        );
    }
}
