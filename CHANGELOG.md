# Changelog

## Unreleased

## 0.1.0 - 2026-11-27

First release.

- **Identification** of the RTC chips at 0x32, 0x51, 0x52, 0x68 and 0x6f, read-only: their register count (where a sequential read wraps), datasheet invariants, and seconds ticking. 17 chips: DS1307, DS1338, DS1337, DS1339, DS3231, DS3232, PCF8523, BQ32000, MCP7940N/MCP7941x, PCF2127/PCF2129, PCF85063A, PCF85063TP, PCF8563, PCF85263A/PCF85363A, RV-3032-C7, RV-3028-C7, RV-8803-C7.
- **Confidence** (certain, probable, possible) and the chip's own "time invalid" flag.
- **`--bind`**: loads the driver's module and instantiates the chip (`new_device`).
