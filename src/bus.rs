//! Read-only access to the devices of an I2C bus.

use std::fmt;

/// Why a read failed.
#[derive(Debug)]
pub enum BusError {
    /// Nothing acknowledged the address: no device there.
    Absent,
    /// The bus or the device failed otherwise.
    Io(std::io::Error),
}

impl fmt::Display for BusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BusError::Absent => f.write_str("no device acknowledged the address"),
            BusError::Io(e) => write!(f, "I2C transfer failed: {e}"),
        }
    }
}

impl std::error::Error for BusError {}

/// An I2C bus, as rtc-probe uses it: register reads only, never a write to
/// a device.
pub trait I2cBus {
    /// Reads `buf.len()` bytes from the device at `address`, starting at
    /// register `register`: the register pointer is written, then the bytes
    /// are read in the same transfer (repeated start), the device
    /// incrementing its pointer after each one.
    fn read(&mut self, address: u8, register: u8, buf: &mut [u8]) -> Result<(), BusError>;
}

impl<B: I2cBus + ?Sized> I2cBus for &mut B {
    fn read(&mut self, address: u8, register: u8, buf: &mut [u8]) -> Result<(), BusError> {
        (**self).read(address, register, buf)
    }
}
