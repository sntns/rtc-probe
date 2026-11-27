//! Identifying the chip at an address from two reads of its registers,
//! about a second apart.

use std::time::Duration;

use crate::bus::BusError;
use crate::bus::I2cBus;
use crate::chips::Check;
use crate::chips::Chip;
use crate::chips::Integrity;
use crate::chips::CHIPS;
use crate::registers::bcd;
use crate::registers::ticked;

/// How many bytes are read from register 0x00: twice the longest wrap
/// known.
pub const READ_LENGTH: usize = 512;

/// The wait between the two reads: a seconds counter changes in between.
pub const TICK: Duration = Duration::from_millis(1100);

/// Two reads of the registers at an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// The address read.
    pub address: u8,
    /// [`READ_LENGTH`] bytes from register 0x00.
    pub first: Vec<u8>,
    /// The same, [`TICK`] later.
    pub second: Vec<u8>,
}

/// How sure an identification is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// Its checks pass and its seconds tick, but its wrap length is not
    /// documented: nothing else rules it out.
    Possible,
    /// Its documented wrap length and its checks match, but its seconds do
    /// not tick: its oscillator is stopped.
    Probable,
    /// Its documented wrap length and its checks match, and its seconds
    /// tick.
    Certain,
}

/// Whether the chip's time can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Oscillator {
    /// Its seconds tick, and it flags nothing.
    Running,
    /// Stopped, or flagged as having stopped (battery out, never set).
    Stopped,
}

/// A chip identified at an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identification {
    /// Where it answers.
    pub address: u8,
    /// What it is.
    pub chip: &'static str,
    /// The name to give the kernel.
    pub device: &'static str,
    /// The kernel module of its driver.
    pub module: &'static str,
    /// How sure that is.
    pub confidence: Confidence,
    /// Whether its time can be trusted.
    pub oscillator: Oscillator,
    /// The wrap length seen, if any.
    pub wrap: Option<usize>,
}

/// What an address holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// Nothing answers.
    Absent,
    /// Something answers that is no known RTC.
    Unknown {
        /// Where it answers.
        address: u8,
        /// The wrap length seen, if any.
        wrap: Option<usize>,
    },
    /// One chip matches best.
    Chip(Identification),
    /// Several chips match as well: none is bound.
    Ambiguous {
        /// Where it answers.
        address: u8,
        /// The chips it could be.
        candidates: Vec<Identification>,
    },
}

/// Reads the registers of every address in `addresses`, twice, `wait`
/// apart: `None` where nothing answers.
pub fn observe(
    bus: &mut impl I2cBus,
    addresses: &[u8],
    wait: impl FnOnce(),
) -> Vec<(u8, Result<Observation, BusError>)> {
    let mut firsts = Vec::new();
    for &address in addresses {
        let mut first = vec![0; READ_LENGTH];
        firsts.push((address, bus.read(address, 0, &mut first).map(|()| first)));
    }
    if firsts.iter().any(|(_, r)| r.is_ok()) {
        wait();
    }
    firsts
        .into_iter()
        .map(|(address, first)| {
            let observation = first.and_then(|first| {
                let mut second = vec![0; READ_LENGTH];
                bus.read(address, 0, &mut second)?;
                Ok(Observation {
                    address,
                    first,
                    second,
                })
            });
            (address, observation)
        })
        .collect()
}

/// The shortest of `wraps` after which `first` repeats itself, registers
/// that changed between the two reads excepted (they may change during a
/// read too: hundredths, seconds).
pub fn wrap_length(first: &[u8], second: &[u8], wraps: &[usize]) -> Option<usize> {
    if first.iter().all(|b| *b == first[0]) {
        return None;
    }
    wraps.iter().copied().find(|&period| {
        if period * 2 > first.len() {
            return false;
        }
        let changing = |register: usize| first[register] != second[register];
        let mut tolerated = 0;
        for register in 0..period {
            let repeats = first[register..]
                .iter()
                .step_by(period)
                .all(|b| *b == first[register]);
            if !repeats {
                if !changing(register) {
                    return false;
                }
                tolerated += 1;
            }
        }
        // A ticking register or two, not a whole register file.
        tolerated <= 2
    })
}

fn check(check: &Check, registers: &[u8]) -> bool {
    let at = |register: u8| registers[usize::from(register)];
    match *check {
        Check::Zero { register, mask } => at(register) & mask == 0,
        Check::Equal {
            register,
            mask,
            value,
        } => at(register) & mask == value,
        Check::Bcd {
            register,
            mask,
            min,
            max,
        } => bcd(at(register) & mask).is_some_and(|v| (min..=max).contains(&v)),
        Check::OneHot { register } => {
            let value = at(register);
            value & 0x80 == 0 && value.count_ones() == 1
        }
        Check::Temperature { register } => (-40..=85).contains(&(at(register) as i8)),
        Check::Mirror { register, of } => at(register) == at(of),
    }
}

fn stopped(integrity: &Integrity, registers: &[u8]) -> bool {
    match *integrity {
        Integrity::StoppedWhenSet { register, bit } => {
            registers[usize::from(register)] & (1 << bit) != 0
        }
        Integrity::StoppedWhenClear { register, bit } => {
            registers[usize::from(register)] & (1 << bit) == 0
        }
    }
}

/// How well `chip` explains `observation`, if it does at all.
pub fn matches(
    chip: &Chip,
    observation: &Observation,
    wrap: Option<usize>,
) -> Option<Identification> {
    if chip.address != observation.address {
        return None;
    }
    let documented = match (chip.wrap, wrap) {
        (Some(expected), Some(seen)) if expected == seen => chip.wrap_confirmed,
        // Another wrap seen: not this chip.
        (Some(_), Some(_)) => return None,
        // A documented wrap not seen: not this chip.
        (Some(_), None) if chip.wrap_confirmed => return None,
        _ => false,
    };
    let both = [&observation.first, &observation.second];
    if !chip
        .checks
        .iter()
        .all(|c| both.iter().all(|registers| check(c, registers)))
    {
        return None;
    }
    let (register, mask) = chip.seconds;
    let seconds = usize::from(register);
    if bcd(observation.first[seconds] & mask).map_or(true, |s| s > 59) {
        return None;
    }
    let ticking = ticked(
        observation.first[seconds],
        observation.second[seconds],
        mask,
    );
    let flagged = chip
        .integrity
        .iter()
        .any(|i| stopped(i, &observation.second));
    let confidence = match (documented, ticking) {
        (true, true) => Confidence::Certain,
        (true, false) => Confidence::Probable,
        (false, true) => Confidence::Possible,
        // Nothing documented seen, nothing ticking: no evidence.
        (false, false) => return None,
    };
    Some(Identification {
        address: observation.address,
        chip: chip.name,
        device: chip.device,
        module: chip.module,
        confidence,
        oscillator: if ticking && !flagged {
            Oscillator::Running
        } else {
            Oscillator::Stopped
        },
        wrap,
    })
}

/// What `observation` holds, among the chips known.
pub fn identify(observation: &Observation) -> Finding {
    let wraps = crate::chips::wraps();
    let wrap = wrap_length(&observation.first, &observation.second, &wraps);
    let mut candidates: Vec<Identification> = CHIPS
        .iter()
        .filter_map(|chip| matches(chip, observation, wrap))
        .collect();
    let Some(best) = candidates.iter().map(|c| c.confidence).max() else {
        return Finding::Unknown {
            address: observation.address,
            wrap,
        };
    };
    candidates.retain(|c| c.confidence == best);
    if candidates.len() == 1 {
        Finding::Chip(candidates.remove(0))
    } else {
        Finding::Ambiguous {
            address: observation.address,
            candidates,
        }
    }
}

/// Every known RTC address on `bus`, identified.
pub fn probe(bus: &mut impl I2cBus, wait: impl FnOnce()) -> Vec<(u8, Result<Finding, BusError>)> {
    let addresses = crate::chips::addresses();
    observe(bus, &addresses, wait)
        .into_iter()
        .map(|(address, observation)| {
            let finding = match observation {
                Ok(observation) => Ok(identify(&observation)),
                Err(BusError::Absent) => Ok(Finding::Absent),
                Err(e) => Err(e),
            };
            (address, finding)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeBus;
    use crate::fake::FakeChip;

    /// A register file `chip` could hold: pseudo-random bytes, then what its
    /// datasheet fixes, its seconds at 10 and its oscillator running.
    fn registers_of(chip: &Chip, seed: u32) -> FakeChip {
        let len = chip.wrap.unwrap_or(35);
        let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        let mut registers: Vec<u8> = (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (state >> 16) as u8
            })
            .collect();
        // Running and not flagged, unless a check needs a flag set (the
        // DS1338 is told from a DS1307 by its OSF bit).
        for integrity in chip.integrity {
            match *integrity {
                Integrity::StoppedWhenSet { register, bit } => {
                    registers[usize::from(register)] &= !(1 << bit)
                }
                Integrity::StoppedWhenClear { register, bit } => {
                    registers[usize::from(register)] |= 1 << bit
                }
            }
        }
        for check in chip.checks {
            match *check {
                Check::Zero { register, mask } => registers[usize::from(register)] &= !mask,
                Check::Equal {
                    register,
                    mask,
                    value,
                } => {
                    let r = &mut registers[usize::from(register)];
                    *r = (*r & !mask) | value;
                }
                Check::Bcd {
                    register,
                    mask,
                    min,
                    ..
                } => {
                    let r = &mut registers[usize::from(register)];
                    *r = (*r & !mask) | (((min / 10) << 4) | (min % 10));
                }
                Check::OneHot { register } => registers[usize::from(register)] = 0x04,
                Check::Temperature { register } => registers[usize::from(register)] = 25,
                Check::Mirror { .. } => {}
            }
        }
        let (seconds, mask) = chip.seconds;
        let r = &mut registers[usize::from(seconds)];
        *r = (*r & !mask) | 0x10;
        // Mirrors last: they copy what the rest became.
        for check in chip.checks {
            if let Check::Mirror { register, of } = *check {
                registers[usize::from(register)] = registers[usize::from(of)];
            }
        }
        // The Mirror of the seconds-adjacent registers stays as set; the
        // seconds tick through FakeChip.
        FakeChip {
            registers,
            wrap_to: 0,
            seconds: Some(chip.seconds),
            running: true,
        }
    }

    fn probe_one(chip: FakeChip, address: u8) -> Finding {
        let mut bus = FakeBus::default().with(address, chip);
        let observations = {
            let mut first = vec![0; READ_LENGTH];
            bus.read(address, 0, &mut first).unwrap();
            bus.tick();
            let mut second = vec![0; READ_LENGTH];
            bus.read(address, 0, &mut second).unwrap();
            Observation {
                address,
                first,
                second,
            }
        };
        identify(&observations)
    }

    #[test]
    fn every_known_chip_is_identified_alone() {
        for chip in CHIPS {
            for seed in 0..20 {
                let finding = probe_one(registers_of(chip, seed), chip.address);
                let Finding::Chip(found) = &finding else {
                    panic!("{} (seed {seed}): {finding:?}", chip.name);
                };
                assert_eq!(found.chip, chip.name, "seed {seed}");
                let expected = if chip.wrap_confirmed {
                    Confidence::Certain
                } else {
                    Confidence::Possible
                };
                assert_eq!(found.confidence, expected, "{} seed {seed}", chip.name);
                let flagged = chip.checks.iter().any(|c| {
                    matches!(c, Check::Equal { register, mask, .. }
                        if chip.integrity.iter().any(|i| matches!(i,
                            Integrity::StoppedWhenSet { register: r, bit } if r == register && mask & (1 << bit) != 0)))
                });
                let expected = if flagged {
                    Oscillator::Stopped
                } else {
                    Oscillator::Running
                };
                assert_eq!(found.oscillator, expected, "{}", chip.name);
            }
        }
    }

    #[test]
    fn a_stopped_chip_is_probable_and_stopped() {
        let ds3231 = CHIPS.iter().find(|c| c.name == "DS3231").unwrap();
        let mut chip = registers_of(ds3231, 1);
        chip.running = false;
        chip.registers[0x0f] |= 0x80;

        let Finding::Chip(found) = probe_one(chip, 0x68) else {
            panic!()
        };

        assert_eq!(found.chip, "DS3231");
        assert_eq!(found.confidence, Confidence::Probable);
        assert_eq!(found.oscillator, Oscillator::Stopped);
    }

    #[test]
    fn a_ticking_chip_with_a_stop_flag_is_reported_stopped() {
        let pcf = CHIPS.iter().find(|c| c.name == "PCF2127/PCF2129").unwrap();
        let mut chip = registers_of(pcf, 3);
        chip.registers[0x03] |= 0x80;

        let Finding::Chip(found) = probe_one(chip, 0x51) else {
            panic!()
        };

        assert_eq!(found.confidence, Confidence::Certain);
        assert_eq!(found.oscillator, Oscillator::Stopped);
    }

    #[test]
    fn an_eeprom_is_no_rtc() {
        // A 256-byte EEPROM at 0x52: no wrap within it known as an RTC's,
        // nothing ticking.
        let mut eeprom = FakeChip::new(256, 0xff);
        for (i, r) in eeprom.registers.iter_mut().enumerate().take(40) {
            *r = (i as u8).wrapping_mul(7);
        }
        assert!(matches!(probe_one(eeprom, 0x52), Finding::Unknown { .. }));
    }

    #[test]
    fn a_blank_eeprom_is_no_rtc() {
        assert!(matches!(
            probe_one(FakeChip::new(256, 0xff), 0x52),
            Finding::Unknown { wrap: None, .. }
        ));
    }

    #[test]
    fn nothing_answering_is_absent() {
        let mut bus = FakeBus::default();
        let findings = probe(&mut bus, || {});
        assert!(findings
            .iter()
            .all(|(_, f)| matches!(f, Ok(Finding::Absent))));
    }

    #[test]
    fn a_whole_bus_is_probed_with_one_wait() {
        let ds3231 = CHIPS.iter().find(|c| c.name == "DS3231").unwrap();
        let pcf = CHIPS.iter().find(|c| c.name == "PCF2127/PCF2129").unwrap();
        let bus = std::cell::RefCell::new(
            FakeBus::default()
                .with(0x68, registers_of(ds3231, 7))
                .with(0x51, registers_of(pcf, 8)),
        );
        let mut waits = 0;
        let findings = {
            let mut handle = Ticking(&bus);
            probe(&mut handle, || {
                waits += 1;
                bus.borrow_mut().tick();
            })
        };
        assert_eq!(waits, 1);
        let chips: Vec<&str> = findings
            .iter()
            .filter_map(|(_, f)| match f {
                Ok(Finding::Chip(i)) => Some(i.chip),
                _ => None,
            })
            .collect();
        assert_eq!(chips, ["PCF2127/PCF2129", "DS3231"]);
    }

    /// A bus shared with the wait closure, which ticks it.
    struct Ticking<'a>(&'a std::cell::RefCell<FakeBus>);

    impl I2cBus for Ticking<'_> {
        fn read(&mut self, address: u8, register: u8, buf: &mut [u8]) -> Result<(), BusError> {
            self.0.borrow_mut().read(address, register, buf)
        }
    }
}
