//! The backlight: the TPS92612's PWM input on GPIO13, PWM channel 1 through Linux's sysfs
//! (`dtoverlay=pwm,pin=13,func=4`). Set on a FIBER (2026-11-19); whether it lights: not seen.

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

/// How soon a failed write is tried again: right after the export, udev may not have given the
/// new files to the group `gpio` yet.
const RETRY: std::time::Duration = std::time::Duration::from_secs(1);

/// How long a write may keep failing before it is said: at the start udev gives the channel's
/// files to the group `gpio` a moment after the agent's first write (seen on a FIBER, 2026-11-19,
/// a second later).
const SAY_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

/// Sets the duty cycles that come in, and tries a failed one again every second until it is
/// written or another comes; says once when it still cannot after [`SAY_AFTER`].
pub async fn drive(mut pwm: Pwm, mut duties: tokio::sync::mpsc::UnboundedReceiver<u8>) {
    let mut said = false;
    let mut failing_since: Option<tokio::time::Instant> = None;
    let mut unwritten: Option<u8> = None;
    loop {
        let next = match unwritten {
            Some(_) => tokio::time::timeout(RETRY, duties.recv()).await.ok(),
            None => Some(duties.recv().await),
        };
        match next {
            Some(None) => return,
            Some(Some(duty)) => unwritten = Some(duty),
            None => {}
        }
        if let Some(duty) = unwritten {
            match pwm.set(duty) {
                Ok(()) => {
                    unwritten = None;
                    said = false;
                    failing_since = None;
                }
                Err(e) => {
                    let since = *failing_since.get_or_insert_with(tokio::time::Instant::now);
                    if !said && since.elapsed() >= SAY_AFTER {
                        eprintln!("the backlight ({}): {e}", pwm.chip.display());
                        said = true;
                    }
                }
            }
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

    #[tokio::test(start_paused = true)]
    async fn a_failed_write_is_tried_again() {
        let dir = TestDir::new("pwm-retry");
        let chip = dir.path().join("pwmchip0");
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let driver = tokio::spawn(drive(Pwm::new(chip.clone(), 1), rx));
        tx.send(80).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        // The channel's files appear, as udev would make them writable.
        std::fs::create_dir_all(chip.join("pwm1")).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert_eq!(
            std::fs::read_to_string(chip.join("pwm1/duty_cycle")).unwrap(),
            "800000"
        );
        drop(tx);
        driver.await.unwrap();
    }
}
