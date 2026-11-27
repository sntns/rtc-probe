//! What two reads of a register file tell: how long it is before a
//! sequential read repeats itself, and which registers count seconds.

/// A BCD byte's value, if it is valid BCD.
pub fn bcd(value: u8) -> Option<u8> {
    let (tens, units) = (value >> 4, value & 0x0f);
    (tens <= 9 && units <= 9).then_some(tens * 10 + units)
}

/// The value a BCD seconds counter holds one second after `value`.
pub fn next_second(seconds: u8) -> u8 {
    (seconds + 1) % 60
}

/// Whether `before` then `after`, read about one second apart, are those of a
/// seconds counter (BCD, masked by `mask`): one or two seconds later (the
/// second may change just after the first read and just before the
/// second).
pub fn ticked(before: u8, after: u8, mask: u8) -> bool {
    match (bcd(before & mask), bcd(after & mask)) {
        (Some(b), Some(a)) if b < 60 && a < 60 => {
            a == next_second(b) || a == next_second(next_second(b))
        }
        _ => false,
    }
}

/// The length after which a sequential read from register 0x00 repeats
/// itself, if it does within `dump`: the chip's register count, when its
/// pointer wraps to 0x00. A register file that repeats with a shorter
/// period than `min` (a blank EEPROM: one byte over and over) gives none.
pub fn wrap_length(dump: &[u8], min: usize) -> Option<usize> {
    if dump.iter().all(|b| *b == dump[0]) {
        return None;
    }
    (min..=dump.len() / 2).find(|&period| {
        dump.iter()
            .zip(&dump[period..])
            .all(|(first, again)| first == again)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcd_values() {
        assert_eq!(bcd(0x59), Some(59));
        assert_eq!(bcd(0x00), Some(0));
        assert_eq!(bcd(0x5a), None);
        assert_eq!(bcd(0xa0), None);
    }

    #[test]
    fn a_seconds_counter_ticks_once_or_twice() {
        assert!(ticked(0x41, 0x42, 0x7f));
        assert!(ticked(0x41, 0x43, 0x7f));
        assert!(ticked(0x59, 0x00, 0x7f));
        assert!(ticked(0xd9, 0x80, 0x7f), "flag bit 7 ignored");
        assert!(!ticked(0x41, 0x41, 0x7f), "stopped");
        assert!(!ticked(0x41, 0x45, 0x7f));
        assert!(!ticked(0x7a, 0x7b, 0x7f), "not BCD");
    }

    #[test]
    fn a_register_file_repeats_after_its_length() {
        let file: Vec<u8> = (0..19u8).collect();
        let dump: Vec<u8> = file.iter().cycle().take(64).copied().collect();
        assert_eq!(wrap_length(&dump, 8), Some(19));
    }

    #[test]
    fn a_blank_or_unrepeated_dump_has_no_length() {
        assert_eq!(wrap_length(&[0xff; 64], 8), None);
        let dump: Vec<u8> = (0..64u8).collect();
        assert_eq!(wrap_length(&dump, 8), None);
    }
}
