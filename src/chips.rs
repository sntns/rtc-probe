//! The RTC chips rtc-probe knows: where they answer, what a sequential read
//! of their registers looks like, and the Linux driver for each.
//!
//! Sources: the manufacturers' datasheets, and the `i2c_device_id` tables
//! and OF compatibles of the Linux drivers (drivers/rtc). A wrap length
//! marked unconfirmed was not stated by the datasheet: it is used to rule a
//! chip out only when another length was seen, never to confirm it.

/// What must hold for a register of a chip, read without writing anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// These bits always read 0 ("not implemented").
    Zero {
        /// The register.
        register: u8,
        /// The bits.
        mask: u8,
    },
    /// These bits read `value`.
    Equal {
        /// The register.
        register: u8,
        /// The bits compared.
        mask: u8,
        /// Their value.
        value: u8,
    },
    /// A BCD value within `min..=max`, once masked.
    Bcd {
        /// The register.
        register: u8,
        /// The bits of the value.
        mask: u8,
        /// The lowest value.
        min: u8,
        /// The highest value.
        max: u8,
    },
    /// Exactly one of bits 0-6 set (a one-hot weekday).
    OneHot {
        /// The register.
        register: u8,
    },
    /// A temperature in whole degrees (two's complement), plausible for a
    /// device in service.
    Temperature {
        /// The register.
        register: u8,
    },
    /// Two registers hold the same value (a mirror).
    Mirror {
        /// The register.
        register: u8,
        /// The register it mirrors.
        of: u8,
    },
}

/// How a chip tells that its time can't be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Integrity {
    /// A bit that, set, says the oscillator stopped or the time is invalid
    /// (OSF, OS, VL, PORF, V2F, OF...).
    StoppedWhenSet {
        /// The register.
        register: u8,
        /// The bit (0-7).
        bit: u8,
    },
    /// A bit that, clear, says the oscillator does not run (OSCRUN).
    StoppedWhenClear {
        /// The register.
        register: u8,
        /// The bit (0-7).
        bit: u8,
    },
}

/// An RTC chip.
#[derive(Debug, Clone, Copy)]
pub struct Chip {
    /// What to call it (`DS3231`, `PCF2127/PCF2129`...).
    pub name: &'static str,
    /// Its 7-bit I2C address.
    pub address: u8,
    /// The name to give the kernel (`new_device`).
    pub device: &'static str,
    /// The kernel module of its driver.
    pub module: &'static str,
    /// After how many registers a sequential read from 0x00 repeats.
    pub wrap: Option<usize>,
    /// Whether `wrap` comes from the datasheet.
    pub wrap_confirmed: bool,
    /// The BCD seconds register, and the mask of its value bits.
    pub seconds: (u8, u8),
    /// A register changing at every read (hundredths of seconds), if any.
    pub fraction: Option<u8>,
    /// What its registers always satisfy.
    pub checks: &'static [Check],
    /// How it flags a time not to trust.
    pub integrity: &'static [Integrity],
}

use Check::*;
use Integrity::*;

/// Every chip known, by address.
pub const CHIPS: &[Chip] = &[
    // 0x68: the Dallas/Maxim family and its look-alikes.
    // One entry for both: their registers differ only by the DS1338's OSF
    // (control bit 5), which a DS1307 always reads 0 and some DS1307
    // modules do set, and which no read tells apart once cleared. The
    // ds1338 driver reads OSF: on a DS1307, that is a bit always 0.
    Chip {
        name: "DS1307/DS1338",
        address: 0x68,
        device: "ds1338",
        module: "rtc-ds1307",
        wrap: Some(64),
        wrap_confirmed: true,
        seconds: (0x00, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x01,
                mask: 0x80,
            },
            Zero {
                register: 0x03,
                mask: 0xf8,
            },
            Zero {
                register: 0x04,
                mask: 0xc0,
            },
            Zero {
                register: 0x05,
                mask: 0xe0,
            },
            // Control: OUT 0 OSF SQWE 0 0 RS1 RS0.
            Zero {
                register: 0x07,
                mask: 0x4c,
            },
            Bcd {
                register: 0x01,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
            Bcd {
                register: 0x04,
                mask: 0x3f,
                min: 1,
                max: 31,
            },
            Bcd {
                register: 0x05,
                mask: 0x1f,
                min: 1,
                max: 12,
            },
        ],
        // CH: the clock is halted (set at first power-up); OSF.
        integrity: &[
            StoppedWhenSet {
                register: 0x00,
                bit: 7,
            },
            StoppedWhenSet {
                register: 0x07,
                bit: 5,
            },
        ],
    },
    Chip {
        name: "DS1337",
        address: 0x68,
        device: "ds1337",
        module: "rtc-ds1307",
        wrap: Some(16),
        wrap_confirmed: true,
        seconds: (0x00, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x00,
                mask: 0x80,
            },
            Zero {
                register: 0x01,
                mask: 0x80,
            },
            // Status: OSF 0 0 0 0 0 A2F A1F.
            Zero {
                register: 0x0f,
                mask: 0x7c,
            },
            Bcd {
                register: 0x01,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
        ],
        integrity: &[StoppedWhenSet {
            register: 0x0f,
            bit: 7,
        }],
    },
    Chip {
        name: "DS1339",
        address: 0x68,
        device: "ds1339",
        module: "rtc-ds1307",
        wrap: Some(17),
        wrap_confirmed: true,
        seconds: (0x00, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x00,
                mask: 0x80,
            },
            Zero {
                register: 0x01,
                mask: 0x80,
            },
            Zero {
                register: 0x0f,
                mask: 0x7c,
            },
            Bcd {
                register: 0x01,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
        ],
        integrity: &[StoppedWhenSet {
            register: 0x0f,
            bit: 7,
        }],
    },
    Chip {
        name: "DS3231",
        address: 0x68,
        device: "ds3231",
        module: "rtc-ds1307",
        wrap: Some(19),
        wrap_confirmed: true,
        seconds: (0x00, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x00,
                mask: 0x80,
            },
            Zero {
                register: 0x01,
                mask: 0x80,
            },
            // Status: OSF 0 0 0 EN32kHz BSY A2F A1F.
            Zero {
                register: 0x0f,
                mask: 0x70,
            },
            // Temperature: 8 bits at 0x11, 2 at the top of 0x12.
            Zero {
                register: 0x12,
                mask: 0x3f,
            },
            Temperature { register: 0x11 },
            Bcd {
                register: 0x01,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
        ],
        integrity: &[StoppedWhenSet {
            register: 0x0f,
            bit: 7,
        }],
    },
    Chip {
        name: "DS3232",
        address: 0x68,
        device: "ds3232",
        module: "rtc-ds3232",
        wrap: Some(256),
        wrap_confirmed: false,
        seconds: (0x00, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x00,
                mask: 0x80,
            },
            Zero {
                register: 0x01,
                mask: 0x80,
            },
            Zero {
                register: 0x12,
                mask: 0x3f,
            },
            Temperature { register: 0x11 },
            Bcd {
                register: 0x01,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
        ],
        integrity: &[StoppedWhenSet {
            register: 0x0f,
            bit: 7,
        }],
    },
    Chip {
        name: "PCF8523",
        address: 0x68,
        device: "pcf8523",
        module: "rtc-pcf8523",
        wrap: Some(20),
        wrap_confirmed: true,
        seconds: (0x03, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x02,
                mask: 0x10,
            },
            Zero {
                register: 0x04,
                mask: 0x80,
            },
            Zero {
                register: 0x05,
                mask: 0xc0,
            },
            Zero {
                register: 0x06,
                mask: 0xc0,
            },
            Zero {
                register: 0x07,
                mask: 0xf8,
            },
            Zero {
                register: 0x08,
                mask: 0xe0,
            },
            Zero {
                register: 0x10,
                mask: 0xf8,
            },
            Zero {
                register: 0x12,
                mask: 0x88,
            },
            Bcd {
                register: 0x04,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
        ],
        // OS: the oscillator stopped.
        integrity: &[StoppedWhenSet {
            register: 0x03,
            bit: 7,
        }],
    },
    Chip {
        name: "BQ32000",
        address: 0x68,
        device: "bq32000",
        module: "rtc-bq32k",
        wrap: None,
        wrap_confirmed: false,
        seconds: (0x00, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x03,
                mask: 0xf8,
            },
            Zero {
                register: 0x04,
                mask: 0xc0,
            },
            Zero {
                register: 0x05,
                mask: 0xe0,
            },
            // CFG2: bit 7 set, low nibble 1010.
            Equal {
                register: 0x09,
                mask: 0x8f,
                value: 0x8a,
            },
            Bcd {
                register: 0x01,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
        ],
        // OF, in the minutes register.
        integrity: &[StoppedWhenSet {
            register: 0x01,
            bit: 7,
        }],
    },
    // 0x6f: Microchip.
    Chip {
        name: "MCP7940N/MCP7941x",
        address: 0x6f,
        device: "mcp7940x",
        module: "rtc-ds1307",
        wrap: Some(32),
        wrap_confirmed: true,
        // ST (start oscillator) in bit 7.
        seconds: (0x00, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x01,
                mask: 0x80,
            },
            Zero {
                register: 0x02,
                mask: 0x80,
            },
            Zero {
                register: 0x03,
                mask: 0xc0,
            },
            Zero {
                register: 0x04,
                mask: 0xc0,
            },
            Zero {
                register: 0x05,
                mask: 0xc0,
            },
            Bcd {
                register: 0x01,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
        ],
        // OSCRUN.
        integrity: &[StoppedWhenClear {
            register: 0x03,
            bit: 5,
        }],
    },
    // 0x51: NXP and Micro Crystal.
    Chip {
        name: "PCF85063TP",
        address: 0x51,
        device: "pcf85063tp",
        module: "rtc-pcf85063",
        wrap: Some(11),
        wrap_confirmed: true,
        seconds: (0x04, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x05,
                mask: 0x80,
            },
            Zero {
                register: 0x06,
                mask: 0xc0,
            },
            Bcd {
                register: 0x05,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
            Bcd {
                register: 0x09,
                mask: 0x1f,
                min: 1,
                max: 12,
            },
        ],
        integrity: &[StoppedWhenSet {
            register: 0x04,
            bit: 7,
        }],
    },
    Chip {
        name: "PCF8563",
        address: 0x51,
        device: "pcf8563",
        module: "rtc-pcf8563",
        wrap: Some(16),
        wrap_confirmed: false,
        seconds: (0x02, 0x7f),
        fraction: None,
        // Its unused bits may read 1: only values.
        checks: &[
            Bcd {
                register: 0x03,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
            Bcd {
                register: 0x04,
                mask: 0x3f,
                min: 0,
                max: 23,
            },
            Bcd {
                register: 0x05,
                mask: 0x3f,
                min: 1,
                max: 31,
            },
            Bcd {
                register: 0x07,
                mask: 0x1f,
                min: 1,
                max: 12,
            },
        ],
        // VL: the voltage dropped, the time is not guaranteed.
        integrity: &[StoppedWhenSet {
            register: 0x02,
            bit: 7,
        }],
    },
    Chip {
        name: "PCF85063A",
        address: 0x51,
        device: "pcf85063a",
        module: "rtc-pcf85063",
        wrap: Some(18),
        wrap_confirmed: true,
        seconds: (0x04, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x05,
                mask: 0x80,
            },
            Zero {
                register: 0x06,
                mask: 0xc0,
            },
            Bcd {
                register: 0x05,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
            Bcd {
                register: 0x09,
                mask: 0x1f,
                min: 1,
                max: 12,
            },
        ],
        integrity: &[StoppedWhenSet {
            register: 0x04,
            bit: 7,
        }],
    },
    Chip {
        name: "PCF2127/PCF2129",
        address: 0x51,
        // Works for the PCF2127, PCF2129 and PCA2129 alike (no SRAM nvmem).
        device: "pcf2129",
        module: "rtc-pcf2127",
        wrap: Some(28),
        wrap_confirmed: true,
        seconds: (0x03, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x04,
                mask: 0x80,
            },
            Zero {
                register: 0x05,
                mask: 0xc0,
            },
            Zero {
                register: 0x06,
                mask: 0xc0,
            },
            Zero {
                register: 0x07,
                mask: 0xf8,
            },
            Zero {
                register: 0x08,
                mask: 0xe0,
            },
            Bcd {
                register: 0x04,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
            Bcd {
                register: 0x08,
                mask: 0x1f,
                min: 1,
                max: 12,
            },
        ],
        // OSF.
        integrity: &[StoppedWhenSet {
            register: 0x03,
            bit: 7,
        }],
    },
    Chip {
        name: "PCF85263A/PCF85363A",
        address: 0x51,
        device: "pcf85363",
        module: "rtc-pcf85363",
        wrap: Some(48),
        wrap_confirmed: true,
        seconds: (0x01, 0x7f),
        fraction: Some(0x00),
        checks: &[
            Zero {
                register: 0x2e,
                mask: 0xfe,
            },
            Bcd {
                register: 0x02,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
            Bcd {
                register: 0x06,
                mask: 0x1f,
                min: 1,
                max: 12,
            },
        ],
        integrity: &[StoppedWhenSet {
            register: 0x01,
            bit: 7,
        }],
    },
    Chip {
        name: "RV-3032-C7",
        address: 0x51,
        device: "rv3032",
        module: "rtc-rv3032",
        wrap: Some(256),
        wrap_confirmed: true,
        seconds: (0x01, 0x7f),
        fraction: Some(0x00),
        checks: &[
            Zero {
                register: 0x01,
                mask: 0x80,
            },
            Zero {
                register: 0x02,
                mask: 0x80,
            },
            Zero {
                register: 0x03,
                mask: 0xc0,
            },
            Zero {
                register: 0x04,
                mask: 0xf8,
            },
            Zero {
                register: 0x05,
                mask: 0xc0,
            },
            Zero {
                register: 0x06,
                mask: 0xe0,
            },
            Bcd {
                register: 0x06,
                mask: 0x1f,
                min: 1,
                max: 12,
            },
        ],
        // VLF, PORF.
        integrity: &[
            StoppedWhenSet {
                register: 0x0d,
                bit: 0,
            },
            StoppedWhenSet {
                register: 0x0d,
                bit: 1,
            },
        ],
    },
    // 0x52.
    Chip {
        name: "RV-3028-C7",
        address: 0x52,
        device: "rv3028",
        module: "rtc-rv3028",
        wrap: Some(64),
        wrap_confirmed: true,
        seconds: (0x00, 0x7f),
        fraction: None,
        checks: &[
            Zero {
                register: 0x00,
                mask: 0x80,
            },
            Zero {
                register: 0x01,
                mask: 0x80,
            },
            Zero {
                register: 0x03,
                mask: 0xf8,
            },
            Bcd {
                register: 0x01,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
            Bcd {
                register: 0x05,
                mask: 0x1f,
                min: 1,
                max: 12,
            },
        ],
        // PORF.
        integrity: &[StoppedWhenSet {
            register: 0x0e,
            bit: 0,
        }],
    },
    // 0x32: Micro Crystal / Epson.
    Chip {
        name: "RV-8803-C7/RX-8803/RX-8900",
        address: 0x32,
        device: "rv8803",
        module: "rtc-rv8803",
        wrap: Some(256),
        wrap_confirmed: true,
        seconds: (0x00, 0x7f),
        fraction: Some(0x10),
        checks: &[
            Zero {
                register: 0x00,
                mask: 0x80,
            },
            OneHot { register: 0x03 },
            // 0x11-0x17 mirror 0x00-0x06 (0x10: hundredths).
            Mirror {
                register: 0x14,
                of: 0x03,
            },
            Mirror {
                register: 0x16,
                of: 0x05,
            },
            Zero {
                register: 0x0e,
                mask: 0xc0,
            },
            Bcd {
                register: 0x01,
                mask: 0x7f,
                min: 0,
                max: 59,
            },
        ],
        // V2F: the data are no longer valid.
        integrity: &[StoppedWhenSet {
            register: 0x0e,
            bit: 1,
        }],
    },
];

/// The addresses where a known chip may answer.
pub fn addresses() -> Vec<u8> {
    let mut addresses: Vec<u8> = CHIPS.iter().map(|c| c.address).collect();
    addresses.sort_unstable();
    addresses.dedup();
    addresses
}

/// The wrap lengths known, shortest first.
pub fn wraps() -> Vec<usize> {
    let mut wraps: Vec<usize> = CHIPS.iter().filter_map(|c| c.wrap).collect();
    wraps.sort_unstable();
    wraps.dedup();
    wraps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_at_one_address_are_told_apart() {
        // Two chips at one address with the same confirmed wrap need a
        // check that tells them apart (DS1307/DS1338: control bit 5).
        for (i, a) in CHIPS.iter().enumerate() {
            for b in &CHIPS[i + 1..] {
                if a.address == b.address && a.wrap == b.wrap && a.wrap_confirmed {
                    assert!(
                        a.checks
                            .iter()
                            .chain(b.checks)
                            .any(|c| matches!(c, Check::Equal { .. })),
                        "{} and {} look alike",
                        a.name,
                        b.name
                    );
                }
            }
        }
    }

    #[test]
    fn registers_checked_are_within_the_chip() {
        for chip in CHIPS {
            let Some(wrap) = chip.wrap else { continue };
            for check in chip.checks {
                let register = match *check {
                    Zero { register, .. }
                    | Equal { register, .. }
                    | Bcd { register, .. }
                    | OneHot { register }
                    | Temperature { register }
                    | Mirror { register, .. } => register,
                };
                assert!(
                    usize::from(register) < wrap,
                    "{}: 0x{register:02x}",
                    chip.name
                );
            }
        }
    }
}
