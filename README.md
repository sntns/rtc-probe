# rtc-probe

[![crates.io](https://img.shields.io/crates/v/rtc-probe.svg)](https://crates.io/crates/rtc-probe)
[![docs.rs](https://img.shields.io/docsrs/rtc-probe)](https://docs.rs/rtc-probe)
[![CI](https://github.com/sntns/rtc-probe/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sntns/rtc-probe/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/rtc-probe.svg)](#license)

Find out which real-time clock chip sits on an I2C bus, without writing to it, and bind the right Linux driver.

```console
$ rtc-probe --bind
i2c-1 0x51: PCF2127/PCF2129 (certain; 28 registers; oscillator running), driver pcf2129 (rtc-pcf2127)
i2c-1 0x51: bound: pcf2129 (rtc-pcf2127-i2c)
```

## Why

On a Raspberry Pi, an RTC module is usually declared in the device tree, for instance with `dtoverlay=i2c-rtc,ds3231`. That works when you know the chip in advance. When the same image runs on boards with different RTC modules, it does not:

- **Declaring several chips** (`i2c-rtc,pcf2127,ds1307,ds3231`) cannot work at one address. Two nodes at 0x68 conflict, and the second one is never created.
- **A driver binds to whatever answers at its address.** These chips have no identification register, so a PCF85063 at 0x51 gets driven as a PCF2127, and a DS3231 at 0x68 as a DS1307. The time may even look right while the registers mean something else. In particular, a stopped-oscillator flag or a dead battery goes unnoticed.
- **Binding the wrong driver can corrupt the chip.** Several drivers write to the chip when they bind: control registers, trickle charger, battery switch-over.

rtc-probe identifies the chip by its behaviour first, then binds the driver for that chip only.

## How it identifies a chip

For every address where a known RTC can answer (0x32, 0x51, 0x52, 0x68, 0x6f), rtc-probe reads 512 bytes from register 0x00 twice, 1.1 s apart, in one transfer each. The only thing it writes is the register pointer.

1. **Register count.** Each chip's address pointer wraps back to 0x00 after its last register, so a sequential read repeats after a length the datasheet gives. Some examples:

   | Chip | Repeats after |
   |---|---|
   | PCF85063TP | 11 |
   | PCF85063A | 18 |
   | DS3231 | 19 |
   | PCF8523 | 20 |
   | PCF2127/2129 | 28 |
   | MCP7940N | 32 |
   | PCF85363 | 48 |
   | DS1307, RV-3028 | 64 |

   That length tells most chips apart even when the oscillator is stopped.
2. **Datasheet invariants.** Bits documented as always 0, a plausible DS3231 temperature, the RV-8803 one-hot weekday and register mirror, and BCD ranges of the time. The BCD ranges are checked only when the chip does not flag its time as invalid: a chip that lost its time holds garbage there.
3. **Ticking.** The seconds register (at its chip-specific address) must have advanced by one or two seconds between the reads. This rejects EEPROMs and other devices that share the addresses.

Each finding gets a confidence:

| Confidence | Meaning |
|---|---|
| **certain** | Documented register count and invariants match, and the seconds tick. |
| **probable** | Documented register count and invariants match, but the seconds do not tick: the oscillator is stopped. Typical of a new chip that was never set, or a dead battery. |

A chip that flags its time as invalid (its OSF, OS, VL... bit set) must have its documented register count to be identified.
| **possible** | The datasheet does not document the register count. The invariants match and the seconds tick. |

rtc-probe also reports the chip's own "time invalid" flag (OSF, OS, VL, PORF, V2F, OSCRUN).

`--bind` only binds a certain or probable chip. It first loads the driver's module, since an initramfs has no udev to do it, then writes `new_device`.

## Chips

| Address | Chips |
|---|---|
| 0x68 | DS1307/DS1338, DS1337, DS1339, DS3231, DS3232*, PCF8523, BQ32000* |
| 0x6f | MCP7940N / MCP7941x |
| 0x51 | PCF2127 / PCF2129 / PCA2129, PCF85063A, PCF85063TP, PCF8563*, PCF85263A / PCF85363A, RV-3032-C7 |
| 0x52 | RV-3028-C7 |
| 0x32 | RV-8803-C7 / RX-8803 / RX-8900 |

\* The datasheet does not state the register count. These chips are identified at most as *possible*, so `--bind` leaves them alone.

Each chip's facts (register map, wrap, invariants, flags) and the Linux device name come from the manufacturer's datasheet and the kernel drivers. They are in [`src/chips.rs`](src/chips.rs), with comments.

## Usage

```console
# modprobe i2c-dev
# rtc-probe                 # identify only
# rtc-probe --bind          # identify, then bind the driver
# rtc-probe --bus 0 --dump  # another bus; print the registers read
```

| Exit status | Meaning |
|---|---|
| 0 | An RTC was identified (and bound, with `--bind`). |
| 1 | No RTC was found. |
| 2 | An error, an ambiguous chip, or a failed bind. |

### At boot, on a Raspberry Pi

1. Keep the I2C bus enabled in `config.txt` (`dtparam=i2c_arm=on`), but **declare no RTC** in the device tree (no `i2c-rtc` overlay). A device already declared at an address cannot be replaced with `new_device`, and rtc-probe reports it.
2. In the initramfs, load the bus drivers, then bind the RTC and set the clock from it:

   ```sh
   modprobe i2c-bcm2835
   modprobe i2c-dev
   rtc-probe --bind && hwclock -s -u
   ```

   Or let the kernel set the clock when the RTC registers (`CONFIG_RTC_HCTOSYS`).

rtc-probe is a single static binary of about 700 KB, with `libc` as its only dependency, so it fits in an initramfs.

## Building

```sh
cargo build --release
# For a Raspberry Pi, static:
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --target armv7-unknown-linux-gnueabihf
```

## Library

The identification runs on any `I2cBus` implementation (`linux::LinuxI2c` for `/dev/i2c-N`):

- `identify::probe` returns a `Finding` for every known address.
- `identify::identify` classifies one pair of reads.

## Contributing a chip

To add a chip, add an entry to `src/chips.rs` with its datasheet facts. The tests build random register files for every chip and check that each one is identified, alone. A register dump from real hardware (`rtc-probe --dump`) is welcome in the pull request.

## Minimum Rust version

1.75.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
