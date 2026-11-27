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

/// The shortest of `wraps` after which `first` repeats itself.
///
/// Tolerated, as not the chip's layout:
/// - up to two registers that changed between the two reads, which may
///   change during a read too (hundredths, seconds);
/// - in a register file of 32 or more (with RAM), one byte that reads
///   differently from one pass to the next without changing between the
///   reads: seen on a real DS1307 module, whose RAM byte 0x15 reads 0xff
///   and 0x00 on alternate passes.
pub fn wrap_length(first: &[u8], second: &[u8], wraps: &[usize]) -> Option<usize> {
    if first.iter().all(|b| *b == first[0]) {
        return None;
    }
    wraps.iter().copied().find(|&period| {
        if period * 2 > first.len() {
            return false;
        }
        let mut changing = 0;
        let mut unstable = 0;
        for register in 0..period {
            let repeats = first[register..]
                .iter()
                .step_by(period)
                .all(|b| *b == first[register]);
            if repeats {
                continue;
            }
            if first[register] != second[register] {
                changing += 1;
            } else {
                unstable += 1;
            }
        }
        changing <= 2 && unstable <= usize::from(period >= 32)
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
    // A chip that flags its time as invalid (oscillator stopped, battery
    // out, never set) holds whatever its registers reset to, or garbage:
    // only its structure is checked then, not its time's values.
    let flagged = chip
        .integrity
        .iter()
        .any(|i| stopped(i, &observation.first) || stopped(i, &observation.second));
    if flagged && !documented {
        return None;
    }
    let both = [&observation.first, &observation.second];
    if !chip
        .checks
        .iter()
        .filter(|c| !(flagged && matches!(c, Check::Bcd { .. })))
        .all(|c| both.iter().all(|registers| check(c, registers)))
    {
        return None;
    }
    let (register, mask) = chip.seconds;
    let seconds = usize::from(register);
    if !flagged && bcd(observation.first[seconds] & mask).map_or(true, |s| s > 59) {
        return None;
    }
    let ticking = ticked(
        observation.first[seconds],
        observation.second[seconds],
        mask,
    );
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
    identify_among(observation, CHIPS)
}

/// What `observation` holds, among `chips`.
pub fn identify_among(observation: &Observation, chips: &[Chip]) -> Finding {
    let mut wraps: Vec<usize> = chips.iter().filter_map(|c| c.wrap).collect();
    wraps.sort_unstable();
    wraps.dedup();
    let wrap = wrap_length(&observation.first, &observation.second, &wraps);
    let mut candidates: Vec<Identification> = chips
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

    /// A chip for the tests: `wrap` registers, seconds at 0x00, no check.
    fn synthetic(name: &'static str, wrap: Option<usize>, confirmed: bool) -> Chip {
        Chip {
            name,
            address: 0x68,
            device: "synthetic",
            module: "rtc-synthetic",
            wrap,
            wrap_confirmed: confirmed,
            seconds: (0x00, 0x7f),
            fraction: None,
            checks: &[],
            integrity: &[],
        }
    }

    /// Two reads of a `wrap`-register chip, its seconds at 0x00 ticking
    /// (or not), the rest pseudo-random.
    fn observation(wrap: usize, ticking: bool) -> Observation {
        let mut chip = FakeChip::new(wrap, 0);
        for (i, r) in chip.registers.iter_mut().enumerate() {
            *r = (i as u8).wrapping_mul(37).wrapping_add(11);
        }
        chip.registers[0] = 0x21;
        chip.seconds = Some((0x00, 0x7f));
        chip.running = ticking;
        let mut bus = FakeBus::default().with(0x68, chip);
        let mut first = vec![0; READ_LENGTH];
        bus.read(0x68, 0, &mut first).unwrap();
        bus.tick();
        let mut second = vec![0; READ_LENGTH];
        bus.read(0x68, 0, &mut second).unwrap();
        Observation {
            address: 0x68,
            first,
            second,
        }
    }

    #[test]
    fn a_register_changing_during_the_read_does_not_hide_the_wrap() {
        // Hundredths at 0x00 change at every byte read; seconds at 0x01
        // roll over between the first and the second repeat.
        let period = 48;
        let mut first: Vec<u8> = (0..READ_LENGTH)
            .map(|i| (i % period) as u8 * 3 + 1)
            .collect();
        for (n, i) in (0..READ_LENGTH).step_by(period).enumerate() {
            first[i] = n as u8;
        }
        first[period + 1] = 0x99;
        let mut second = first.clone();
        second[0] = 0x50;
        second[1] = 0x42;
        assert_eq!(wrap_length(&first, &second, &[11, 16, 48, 64]), Some(48));
    }

    #[test]
    fn a_register_that_differs_without_changing_breaks_a_short_wrap() {
        let period = 28;
        let mut first: Vec<u8> = (0..READ_LENGTH)
            .map(|i| (i % period) as u8 * 5 + 2)
            .collect();
        first[period + 7] = 0xee;
        let second = first.clone();
        assert_eq!(wrap_length(&first, &second, &[28, 64]), None);
    }

    #[test]
    fn one_unstable_byte_is_tolerated_in_ram_not_two() {
        let period = 64;
        let mut first: Vec<u8> = (0..READ_LENGTH)
            .map(|i| ((i % period) as u8).wrapping_mul(5).wrapping_add(2))
            .collect();
        first[period + 0x15] = 0x00;
        let second = first.clone();
        assert_eq!(wrap_length(&first, &second, &[28, 64]), Some(64));

        first[period + 0x20] = 0x00;
        let second = first.clone();
        assert_eq!(wrap_length(&first, &second, &[28, 64]), None);
    }

    #[test]
    fn many_changing_registers_are_no_wrap() {
        // A device whose registers all change (a sensor) does not wrap.
        let first: Vec<u8> = (0..READ_LENGTH).map(|i| (i * 7 % 251) as u8).collect();
        let second: Vec<u8> = first.iter().map(|b| b.wrapping_add(1)).collect();
        assert_eq!(wrap_length(&first, &second, &[11, 16, 19, 28]), None);
    }

    #[test]
    fn the_wrap_seen_rules_other_chips_out() {
        let chips = [
            synthetic("nineteen", Some(19), true),
            synthetic("twenty-eight", Some(28), true),
        ];

        let Finding::Chip(found) = identify_among(&observation(28, true), &chips) else {
            panic!()
        };

        assert_eq!(found.chip, "twenty-eight");
        assert_eq!(found.wrap, Some(28));
    }

    #[test]
    fn a_documented_wrap_beats_an_undocumented_one() {
        let chips = [
            synthetic("documented", Some(19), true),
            synthetic("undocumented", None, false),
        ];

        let Finding::Chip(found) = identify_among(&observation(19, true), &chips) else {
            panic!()
        };

        assert_eq!(found.chip, "documented");
        assert_eq!(found.confidence, Confidence::Certain);
    }

    #[test]
    fn an_undocumented_chip_that_does_not_tick_is_unknown() {
        let chips = [synthetic("undocumented", None, false)];

        assert!(matches!(
            identify_among(&observation(35, false), &chips),
            Finding::Unknown { .. }
        ));
    }

    #[test]
    fn a_documented_wrap_not_seen_rules_the_chip_out() {
        let chips = [synthetic("nineteen", Some(19), true)];

        assert!(matches!(
            identify_among(&observation(35, true), &chips),
            Finding::Unknown { .. }
        ));
    }

    #[test]
    fn chips_that_match_as_well_are_ambiguous() {
        let chips = [
            synthetic("one", Some(19), true),
            synthetic("other", Some(19), true),
        ];

        let Finding::Ambiguous { candidates, .. } = identify_among(&observation(19, true), &chips)
        else {
            panic!()
        };

        let names: Vec<&str> = candidates.iter().map(|c| c.chip).collect();
        assert_eq!(names, ["one", "other"]);
    }

    #[test]
    fn invalid_seconds_rule_a_chip_out() {
        let chips = [synthetic("nineteen", Some(19), true)];
        let mut seen = observation(19, true);
        seen.first[0] = 0x7a;

        assert!(matches!(
            identify_among(&seen, &chips),
            Finding::Unknown { .. }
        ));
    }

    /// A bus failing at some addresses.
    struct Failing {
        bus: FakeBus,
        failing: Vec<u8>,
        /// Fails only from the n-th read on.
        after: usize,
        reads: usize,
    }

    impl I2cBus for Failing {
        fn read(&mut self, address: u8, register: u8, buf: &mut [u8]) -> Result<(), BusError> {
            self.reads += 1;
            if self.failing.contains(&address) && self.reads > self.after {
                return Err(BusError::Io(std::io::Error::from_raw_os_error(libc::EIO)));
            }
            self.bus.read(address, register, buf)
        }
    }

    #[test]
    fn an_address_failing_does_not_stop_the_others() {
        let pcf = CHIPS.iter().find(|c| c.name == "PCF2127/PCF2129").unwrap();
        let mut bus = Failing {
            bus: FakeBus::default().with(0x51, registers_of(pcf, 2)),
            failing: vec![0x68],
            after: 0,
            reads: 0,
        };

        let observations = observe(&mut bus, &[0x51, 0x68], || {});

        assert!(observations[0].1.is_ok());
        assert!(matches!(observations[1].1, Err(BusError::Io(_))));
    }

    #[test]
    fn a_second_read_failing_is_reported() {
        let pcf = CHIPS.iter().find(|c| c.name == "PCF2127/PCF2129").unwrap();
        let mut bus = Failing {
            bus: FakeBus::default().with(0x51, registers_of(pcf, 2)),
            failing: vec![0x51],
            after: 1,
            reads: 0,
        };

        let observations = observe(&mut bus, &[0x51], || {});

        assert!(matches!(observations[0].1, Err(BusError::Io(_))));
    }

    #[test]
    fn nothing_answering_means_no_wait() {
        let mut waited = false;
        let observations = observe(&mut FakeBus::default(), &[0x51, 0x68], || waited = true);

        assert!(!waited);
        assert!(observations
            .iter()
            .all(|(_, o)| matches!(o, Err(BusError::Absent))));
    }

    /// A PCF2127 module on a Raspberry Pi 3 (bus 1, 0x51), its 28 registers
    /// as read at 10:11:21 UTC: Control_1-3, the time, alarms disabled
    /// (bit 7), CLKOUT, watchdog, time stamps, aging offset 8.
    const REAL_PCF2127: [u8; 28] = [
        0x00, 0x04, 0x00, 0x21, 0x11, 0x09, 0x27, 0x05, 0x11, 0x26, 0x80, 0x80, 0x80, 0x80, 0x80,
        0x20, 0xc2, 0x00, 0x88, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00,
    ];

    #[test]
    fn a_real_pcf2127_is_identified() {
        let chip = FakeChip {
            registers: REAL_PCF2127.to_vec(),
            wrap_to: 0,
            seconds: Some((0x03, 0x7f)),
            running: true,
        };

        let Finding::Chip(found) = probe_one(chip, 0x51) else {
            panic!()
        };

        assert_eq!(found.chip, "PCF2127/PCF2129");
        assert_eq!(found.confidence, Confidence::Certain);
        assert_eq!(found.oscillator, Oscillator::Running);
        assert_eq!(found.wrap, Some(28));
    }

    #[test]
    fn a_real_pcf2127_read_one_register_late_is_no_rtc() {
        // What a combined transfer (repeated START) returns from it: the
        // registers from 0x01. Nothing known matches it, rather than a wrong
        // chip.
        let mut late = REAL_PCF2127.to_vec();
        late.rotate_left(1);
        let chip = FakeChip {
            registers: late,
            wrap_to: 0,
            seconds: Some((0x02, 0x7f)),
            running: true,
        };

        assert!(matches!(
            probe_one(chip, 0x51),
            Finding::Unknown { wrap: Some(28), .. }
        ));
    }

    /// A DS1307 module on a Raspberry Pi 3 (bus 1, 0x68), its 64 registers
    /// as read at 09:46:26 UTC (control 0x93: OUT, SQWE, 32.768 kHz). Its RAM
    /// byte 0x15 reads 0xff and 0x00 on alternate passes.
    const REAL_DS1307: [u8; 64] = [
        0x26, 0x46, 0x09, 0x06, 0x27, 0x11, 0x26, 0x93, 0xff, 0xbf, 0xfe, 0xff, 0xff, 0xff, 0xdf,
        0xfe, 0xff, 0x77, 0xff, 0xbf, 0xff, 0xff, 0xef, 0xfe, 0xff, 0xfb, 0xff, 0xff, 0xff, 0xff,
        0xdf, 0xff, 0x80, 0x04, 0x00, 0x08, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00,
        0x00, 0x00, 0x04, 0x80, 0x60, 0x00, 0x00, 0x80, 0x10, 0x08, 0x00, 0x00, 0x22, 0x00, 0x00,
        0x00, 0x00, 0x14, 0x20,
    ];

    /// Its registers read from 0x00 as it returns them: byte 0x15 of odd
    /// passes 0x00.
    fn ds1307_read(seconds: u8) -> Vec<u8> {
        let mut read: Vec<u8> = REAL_DS1307
            .iter()
            .copied()
            .cycle()
            .take(READ_LENGTH)
            .collect();
        for pass in (0..READ_LENGTH / 64).filter(|p| p % 2 == 1) {
            read[pass * 64 + 0x15] = 0x00;
        }
        for pass in 0..READ_LENGTH / 64 {
            read[pass * 64] = seconds;
        }
        read
    }

    #[test]
    fn a_real_ds1307_is_identified() {
        let observation = Observation {
            address: 0x68,
            first: ds1307_read(0x26),
            second: ds1307_read(0x27),
        };

        let Finding::Chip(found) = identify(&observation) else {
            panic!("{:?}", identify(&observation))
        };

        assert_eq!(found.chip, "DS1307/DS1338");
        assert_eq!(found.confidence, Confidence::Certain);
        assert_eq!(found.oscillator, Oscillator::Running);
        assert_eq!(found.wrap, Some(64));
    }

    /// Another DS1307 module (battery-backed, control at its power-on 0x03),
    /// read at 10:04:19 UTC.
    const REAL_DS1307_BATTERY: [u8; 64] = [
        0x19, 0x04, 0x10, 0x06, 0x27, 0x11, 0x26, 0x03, 0x40, 0x00, 0x00, 0x00, 0x08, 0x00, 0x11,
        0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x14, 0x08, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x02, 0x04, 0x00, 0x00, 0x00, 0x40, 0x40, 0x02, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00,
        0x01, 0x10, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x01,
    ];

    #[test]
    fn another_real_ds1307_is_identified() {
        let chip = FakeChip {
            registers: REAL_DS1307_BATTERY.to_vec(),
            wrap_to: 0,
            seconds: Some((0x00, 0x7f)),
            running: true,
        };

        let Finding::Chip(found) = probe_one(chip, 0x68) else {
            panic!()
        };

        assert_eq!(found.chip, "DS1307/DS1338");
        assert_eq!(found.confidence, Confidence::Certain);
        assert_eq!(found.oscillator, Oscillator::Running);
    }

    /// The PCF2127 module after a power cut without a battery, read at
    /// 11:05 UTC: OSF set, Control_1 back to its reset 0x08, the time
    /// garbage (hours 0x0b, day and month 0) - but its seconds tick.
    const REAL_PCF2127_LOST: [u8; 28] = [
        0x08, 0x00, 0x04, 0x95, 0x06, 0x0b, 0x00, 0x00, 0x00, 0x01, 0xc0, 0xc6, 0xa0, 0x81, 0x80,
        0x00, 0x03, 0x00, 0x08, 0x40, 0x00, 0x0a, 0x10, 0x00, 0x02, 0x08, 0x00, 0x00,
    ];

    #[test]
    fn a_real_pcf2127_that_lost_its_time_is_identified_and_not_trusted() {
        let chip = FakeChip {
            registers: REAL_PCF2127_LOST.to_vec(),
            wrap_to: 0,
            seconds: Some((0x03, 0x7f)),
            running: true,
        };

        let Finding::Chip(found) = probe_one(chip, 0x51) else {
            panic!()
        };

        assert_eq!(found.chip, "PCF2127/PCF2129");
        assert_eq!(found.confidence, Confidence::Certain);
        assert_eq!(found.oscillator, Oscillator::Stopped);
    }

    #[test]
    fn a_halted_ds1307_with_garbage_is_probable() {
        // CH set, not ticking, the date out of range.
        let mut registers = REAL_DS1307_BATTERY.to_vec();
        registers[0x00] = 0x80;
        registers[0x04] = 0x00;
        registers[0x05] = 0x00;
        let chip = FakeChip {
            registers,
            wrap_to: 0,
            seconds: Some((0x00, 0x7f)),
            running: false,
        };

        let Finding::Chip(found) = probe_one(chip, 0x68) else {
            panic!()
        };

        assert_eq!(found.chip, "DS1307/DS1338");
        assert_eq!(found.confidence, Confidence::Probable);
        assert_eq!(found.oscillator, Oscillator::Stopped);
    }

    #[test]
    fn a_flagged_chip_needs_its_documented_wrap() {
        // A PCF8563 (undocumented wrap) with VL set and its time garbage:
        // nothing documented vouches for it.
        let mut chip = FakeChip::new(16, 0);
        for (i, r) in chip.registers.iter_mut().enumerate() {
            *r = (i as u8).wrapping_mul(29).wrapping_add(3);
        }
        chip.registers[0x02] = 0x80 | 0x12;
        chip.registers[0x07] = 0x00;
        chip.seconds = Some((0x02, 0x7f));

        assert!(matches!(probe_one(chip, 0x51), Finding::Unknown { .. }));
    }

    /// A bus shared with the wait closure, which ticks it.
    struct Ticking<'a>(&'a std::cell::RefCell<FakeBus>);

    impl I2cBus for Ticking<'_> {
        fn read(&mut self, address: u8, register: u8, buf: &mut [u8]) -> Result<(), BusError> {
            self.0.borrow_mut().read(address, register, buf)
        }
    }
}
