//! A simulated I2C bus for tests: chips whose register files wrap after a
//! given length, and whose seconds register ticks.

use std::collections::BTreeMap;

use crate::bus::BusError;
use crate::bus::I2cBus;

/// A simulated chip: its register file, and how a sequential read wraps.
#[derive(Debug, Clone)]
pub struct FakeChip {
    /// The registers, from 0x00.
    pub registers: Vec<u8>,
    /// After the last register, the pointer goes back to this one
    /// (usually 0x00).
    pub wrap_to: u8,
    /// The BCD seconds register, its mask, and whether it ticks.
    pub seconds: Option<(u8, u8)>,
    /// Whether its seconds tick.
    pub running: bool,
}

impl FakeChip {
    /// A chip of `len` registers, all `fill`, wrapping to 0x00.
    pub fn new(len: usize, fill: u8) -> Self {
        Self {
            registers: vec![fill; len],
            wrap_to: 0,
            seconds: None,
            running: true,
        }
    }

    /// Sets register `register` to `value`.
    pub fn set(mut self, register: u8, value: u8) -> Self {
        self.registers[register as usize] = value;
        self
    }

    /// Makes `register` (masked by `mask`) the BCD seconds, ticking or not.
    pub fn seconds(mut self, register: u8, mask: u8, running: bool) -> Self {
        self.seconds = Some((register, mask));
        self.running = running;
        self
    }

    fn tick(&mut self) {
        let Some((register, mask)) = self.seconds else {
            return;
        };
        if !self.running {
            return;
        }
        let value = &mut self.registers[register as usize];
        let flags = *value & !mask;
        let bcd = *value & mask;
        let seconds = (bcd >> 4) * 10 + (bcd & 0x0f);
        let next = (seconds + 1) % 60;
        *value = flags | ((next / 10) << 4) | (next % 10);
    }
}

/// A simulated bus: chips by address. Time advances one second with
/// [`FakeBus::tick`].
#[derive(Debug, Clone, Default)]
pub struct FakeBus {
    /// The chips, by address.
    pub chips: BTreeMap<u8, FakeChip>,
}

impl FakeBus {
    /// Adds `chip` at `address`.
    pub fn with(mut self, address: u8, chip: FakeChip) -> Self {
        self.chips.insert(address, chip);
        self
    }

    /// One second passes on every chip.
    pub fn tick(&mut self) {
        for chip in self.chips.values_mut() {
            chip.tick();
        }
    }
}

impl I2cBus for FakeBus {
    fn read(&mut self, address: u8, register: u8, buf: &mut [u8]) -> Result<(), BusError> {
        let chip = self.chips.get(&address).ok_or(BusError::Absent)?;
        let len = chip.registers.len();
        let mut pointer = usize::from(register) % len;
        for byte in buf.iter_mut() {
            *byte = chip.registers[pointer];
            pointer += 1;
            if pointer >= len {
                pointer = usize::from(chip.wrap_to);
            }
        }
        Ok(())
    }
}
