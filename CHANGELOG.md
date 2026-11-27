# Changelog

## Unreleased

## 0.1.2 - 2026-11-27

Validated on two DS1307 modules on a Raspberry Pi 3.

- **Fixed:** a register file of 32 or more registers (with RAM) may have one byte that reads differently from one pass to the next. The DS1307 module's RAM byte 0x15 reads 0xff and 0x00 on alternate passes, which hid its 64 registers and left it unidentified.
- **`--dump`** prints 256 bytes instead of 64.
- **Tests:** the registers of two real DS1307 modules.

## 0.1.1 - 2026-11-27

Validated on a PCF2127 module on a Raspberry Pi 3.

- **Fixed:** the register pointer is written in a transfer of its own, then the registers are read in another. A PCF2127 answers a combined transfer (repeated START) from the register after the one written, so 0.1.0 could not identify it.
- **Fixed:** on a Raspberry Pi (i2c-bcm2835), an address where nothing answers fails with EIO. 0.1.0 reported it as a bus error and exited with 2.
- **Tests:** the registers of a real PCF2127, read both ways.

## 0.1.0 - 2026-11-27

First release.

- **Identification** of the RTC chips at 0x32, 0x51, 0x52, 0x68 and 0x6f, read-only: their register count (where a sequential read wraps), datasheet invariants, and seconds ticking. 17 chips: DS1307, DS1338, DS1337, DS1339, DS3231, DS3232, PCF8523, BQ32000, MCP7940N/MCP7941x, PCF2127/PCF2129, PCF85063A, PCF85063TP, PCF8563, PCF85263A/PCF85363A, RV-3032-C7, RV-3028-C7, RV-8803-C7.
- **Confidence** (certain, probable, possible) and the chip's own "time invalid" flag.
- **`--bind`**: loads the driver's module and instantiates the chip (`new_device`).
