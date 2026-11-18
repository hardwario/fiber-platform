//! The backlight: the TPS92612's PWM input on GPIO13, PWM channel 1 through Linux's sysfs
//! (`dtoverlay=pwm,pin=13,func=4`). Not run on a FIBER yet.

use std::io;
use std::path::PathBuf;

/// 1 kHz: well above what the eye sees.
pub const PERIOD_NS: u64 = 1_000_000;

/// One PWM channel under a `pwmchipN` directory.
pub struct Pwm {
    chip: PathBuf,
    channel: u32,
    set_up: bool,
}

impl Pwm {
    pub fn new(chip: PathBuf, channel: u32) -> Pwm {
        Pwm {
            chip,
            channel,
            set_up: false,
        }
    }

    /// Sets the duty cycle, 0 to 100 %; exports and sets up the channel the first time.
    pub fn set(&mut self, duty: u8) -> io::Result<()> {
        let dir = self.chip.join(format!("pwm{}", self.channel));
        if !dir.exists() {
            std::fs::write(self.chip.join("export"), self.channel.to_string())?;
        }
        if !self.set_up {
            // The duty cycle first: it may never be longer than the period.
            std::fs::write(dir.join("duty_cycle"), "0")?;
            std::fs::write(dir.join("period"), PERIOD_NS.to_string())?;
            std::fs::write(dir.join("enable"), "1")?;
            self.set_up = true;
        }
        let duty_ns = PERIOD_NS * u64::from(duty.min(100)) / 100;
        std::fs::write(dir.join("duty_cycle"), duty_ns.to_string())
    }
}

/// Sets the duty cycles that come in; says once when it cannot.
pub async fn drive(mut pwm: Pwm, mut duties: tokio::sync::mpsc::UnboundedReceiver<u8>) {
    let mut said = false;
    while let Some(duty) = duties.recv().await {
        match pwm.set(duty) {
            Ok(()) => said = false,
            Err(e) if !said => {
                eprintln!("the backlight ({}): {e}", pwm.chip.display());
                said = true;
            }
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    #[test]
    fn a_channel_set_up_and_set() {
        let dir = TestDir::new("pwm");
        let chip = dir.path().join("pwmchip0");
        // As Linux shows a channel once it is exported.
        std::fs::create_dir_all(chip.join("pwm1")).unwrap();
        let mut pwm = Pwm::new(chip.clone(), 1);
        pwm.set(80).unwrap();
        let read = |name: &str| std::fs::read_to_string(chip.join("pwm1").join(name)).unwrap();
        assert_eq!(read("period"), "1000000");
        assert_eq!(read("enable"), "1");
        assert_eq!(read("duty_cycle"), "800000");
        pwm.set(16).unwrap();
        assert_eq!(read("duty_cycle"), "160000");
        // Not exported, and the export cannot be written: an error, said once by `drive`.
        assert!(Pwm::new(dir.path().join("none"), 1).set(50).is_err());
    }
}
