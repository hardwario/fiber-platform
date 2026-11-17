//! The flasher against a simulated chip: what it writes, what it refuses, and that a write cut
//! short leaves a chip that waits in its bootloader.

// Tests may unwrap (`clippy.toml`), but clippy sees that only inside `#[test]` functions, and this
// file has helpers too.
#![allow(clippy::unwrap_used)]

use fiber_flasher::sim::{Running, Sim};
use fiber_flasher::{Chip, Error, Flasher, PortLock, Progress, STM32C031, STM32WL55, check_image};

/// A firmware for `chip` of `len` bytes: a stack in RAM, a reset vector into it, a pattern.
fn image(chip: &Chip, len: usize) -> Vec<u8> {
    let mut image: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
    image[..4].copy_from_slice(&(chip.ram_end).to_le_bytes());
    image[4..8].copy_from_slice(&(chip.flash_start + 0x101).to_le_bytes());
    image
}

fn flasher(sim: &Sim, chip: Chip) -> Flasher<Sim, Sim> {
    Flasher::new(sim.clone(), sim.clone(), chip)
}

fn flash(sim: &Sim, chip: Chip, image: &[u8]) -> (Result<(), Error>, Vec<Progress>) {
    let mut seen = Vec::new();
    let result = flasher(sim, chip).flash(image, &mut |p| seen.push(p));
    (result, seen)
}

#[test]
fn it_writes_the_image_and_starts_it() {
    let sim = Sim::new(STM32C031);
    let image = image(&STM32C031, 5000);
    let (result, progress) = flash(&sim, STM32C031, &image);
    result.unwrap();
    let chip = sim.0.borrow();
    assert_eq!(&chip.flash[..image.len()], &image[..]);
    // The last block is padded to the flash's word with 0xFF, and the rest stays erased.
    assert!(chip.flash[image.len()..].iter().all(|b| *b == 0xFF));
    assert_eq!(chip.running, Running::Firmware);
    assert!(!chip.boot0, "BOOT0 is left low");
    assert!(
        !chip.even_parity,
        "the port is back at no parity, for the firmware"
    );
    assert!(progress.contains(&Progress::Identified { product_id: 0x453 }));
    assert!(progress.contains(&Progress::Erasing { pages: 3 }));
    // The vector table is written after everything else was written and compared.
    let vectors = progress
        .iter()
        .position(|p| *p == Progress::WritingVectorTable)
        .unwrap();
    assert!(
        progress[..vectors]
            .iter()
            .any(|p| matches!(p, Progress::Verifying { .. }))
    );
    assert_eq!(progress.last(), Some(&Progress::Starting));
}

#[test]
fn it_reads_what_the_chip_is() {
    let sim = Sim::new(STM32WL55);
    let mut f = flasher(&sim, STM32WL55);
    f.enter(&mut |_| {}).unwrap();
    let identity = f.identify().unwrap();
    assert_eq!(identity.product_id, 0x497);
    assert_eq!(identity.version, 0x31);
    assert!(identity.commands.contains(&0x44));
    assert_eq!(identity.option_bytes.rdp(), 0xAA);
    assert!(!identity.option_bytes.readout_protected());
}

#[test]
fn another_chip_is_refused_before_anything_is_erased() {
    let sim = Sim::new(STM32WL55);
    sim.0.borrow_mut().flash[..16].copy_from_slice(&[0x11; 16]);
    let (result, _) = flash(&sim, STM32C031, &image(&STM32C031, 1024));
    assert!(
        matches!(
            result,
            Err(Error::WrongChip {
                found: 0x497,
                expected: 0x453
            })
        ),
        "{result:?}"
    );
    assert_eq!(&sim.0.borrow().flash[..16], &[0x11; 16]);
}

#[test]
fn readout_protection_is_refused() {
    let sim = Sim::new(STM32C031);
    sim.0.borrow_mut().option_bytes = 0xFFFF_FE00;
    let (result, _) = flash(&sim, STM32C031, &image(&STM32C031, 1024));
    assert!(
        matches!(result, Err(Error::ReadoutProtected { .. })),
        "{result:?}"
    );
    assert_eq!(sim.0.borrow().writes, 0);
}

#[test]
fn a_bad_image_is_refused_before_the_chip_is_touched() {
    let chip = STM32C031;
    let mut bad_stack = image(&chip, 1024);
    bad_stack[..4].copy_from_slice(&0x0800_0000u32.to_le_bytes());
    let mut even_vector = image(&chip, 1024);
    even_vector[4..8].copy_from_slice(&(chip.flash_start + 0x100).to_le_bytes());
    let mut vector_outside = image(&chip, 1024);
    vector_outside[4..8].copy_from_slice(&(chip.flash_start + 0x2001).to_le_bytes());
    for bad in [
        vec![0u8; 4],
        image(&chip, 33 * 1024),
        bad_stack,
        even_vector,
        vector_outside,
    ] {
        assert!(matches!(check_image(&chip, &bad), Err(Error::BadImage(_))));
        let sim = Sim::new(chip);
        let (result, progress) = flash(&sim, chip, &bad);
        assert!(matches!(result, Err(Error::BadImage(_))));
        assert!(progress.is_empty(), "nothing is done for a bad image");
        assert_eq!(sim.0.borrow().attempts, 0);
    }
}

#[test]
fn it_tries_again_holding_boot0_longer() {
    let sim = Sim::new(STM32C031);
    sim.0.borrow_mut().silent_attempts = 2;
    let (result, progress) = flash(&sim, STM32C031, &image(&STM32C031, 600));
    result.unwrap();
    let attempts: Vec<_> = progress
        .iter()
        .filter(|p| matches!(p, Progress::Entering { .. }))
        .collect();
    assert_eq!(attempts.len(), 3);
    let sleeps = sim.0.borrow().sleeps.clone();
    assert_eq!(sleeps, [20, 50, 20, 200, 20, 500]);
}

#[test]
fn a_bootloader_that_never_answers() {
    let sim = Sim::new(STM32C031);
    sim.0.borrow_mut().silent_attempts = 99;
    let (result, _) = flash(&sim, STM32C031, &image(&STM32C031, 600));
    assert!(
        matches!(result, Err(Error::NoAnswer { attempts: 3 })),
        "{result:?}"
    );
    assert!(!sim.0.borrow().boot0);
}

#[test]
fn a_write_cut_short_leaves_a_chip_waiting_in_its_bootloader() {
    let chip = STM32C031;
    let sim = Sim::new(chip);
    // An old firmware runs; the power goes in the middle of writing the new one.
    let old = image(&chip, 3000);
    flash(&sim, chip, &old).0.unwrap();
    sim.0.borrow_mut().power_cycle();
    assert_eq!(sim.0.borrow().running, Running::Firmware);
    let writes = sim.0.borrow().writes;
    sim.0.borrow_mut().power_cut_after_writes = Some(writes + 4);
    let new = image(&chip, 6000);
    let (result, _) = flash(&sim, chip, &new);
    assert!(result.is_err());
    // After the power comes back, with BOOT0 low, the chip waits in its bootloader: the first
    // block, with the vector table, was never written.
    {
        let mut s = sim.0.borrow_mut();
        s.power_cut_after_writes = None;
        s.boot0 = false;
        s.power_cycle();
        assert_eq!(s.running, Running::Bootloader { listening: false });
        assert!(s.flash[..256].iter().all(|b| *b == 0xFF));
    }
    // And it is written whole the next time.
    flash(&sim, chip, &new).0.unwrap();
    assert_eq!(&sim.0.borrow().flash[..new.len()], &new[..]);
}

#[test]
fn after_a_first_write_a_reset_lands_in_the_bootloader_again() {
    let chip = STM32WL55;
    let sim = Sim::new(chip);
    let mut f = flasher(&sim, chip);
    f.flash(&image(&chip, 2048), &mut |_| {}).unwrap();
    assert_eq!(sim.0.borrow().running, Running::Firmware, "Go started it");
    // The empty flag is only looked at again at power-on: that is why the flasher leaves by Go.
    f.reset_to_firmware().unwrap();
    assert_eq!(sim.0.borrow().running, Running::Bootloader { listening: false });
    sim.0.borrow_mut().power_cycle();
    assert_eq!(sim.0.borrow().running, Running::Firmware);
}

#[test]
fn what_reads_back_wrong_stops_it_before_the_vector_table() {
    let chip = STM32C031;
    let sim = Sim::new(chip);
    sim.0.borrow_mut().bad_read_at = Some(chip.flash_start + 700);
    let (result, progress) = flash(&sim, chip, &image(&chip, 1500));
    assert!(
        matches!(result, Err(Error::Mismatch { address }) if address == chip.flash_start + 700),
        "{result:?}"
    );
    assert!(!progress.contains(&Progress::WritingVectorTable));
    assert!(sim.0.borrow().flash[..256].iter().all(|b| *b == 0xFF));
}

#[test]
fn a_port_held_by_another() {
    let dir = std::env::temp_dir().join(format!("fiber-flasher-lock-{}", std::process::id()));
    let first = PortLock::take(&dir, "/dev/ttyAMA4").unwrap();
    let second = PortLock::take(&dir, "/dev/ttyAMA4");
    assert_eq!(
        second.err().map(|e| e.kind()),
        Some(std::io::ErrorKind::WouldBlock)
    );
    drop(first);
    PortLock::take(&dir, "/dev/ttyAMA4").unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
