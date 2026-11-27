//! Identify the real-time clock chip on an I2C bus, and bind its Linux
//! driver.

#![warn(missing_docs)]

pub mod bus;
pub mod chips;
pub mod fake;
pub mod identify;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod registers;
