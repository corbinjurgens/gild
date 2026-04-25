use std::fmt;

use chrono::DateTime;

pub const SECONDS_PER_DAY: i64 = 86_400;
pub const SECONDS_PER_WEEK: i64 = 604_800;

pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub struct Sep(pub usize);

impl fmt::Display for Sep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0u8; 26];
        let mut pos = buf.len();
        let mut n = self.0;
        let mut digits = 0u8;
        loop {
            pos -= 1;
            buf[pos] = b'0' + (n % 10) as u8;
            n /= 10;
            digits += 1;
            if n == 0 {
                break;
            }
            if digits % 3 == 0 {
                pos -= 1;
                buf[pos] = b',';
            }
        }
        // SAFETY: buf only contains ASCII digits (b'0'..=b'9') and b','
        f.write_str(unsafe { std::str::from_utf8_unchecked(&buf[pos..]) })
    }
}

pub fn fmt_date(timestamp: i64, format: &str) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format(format).to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_num_zero() {
        assert_eq!(Sep(0).to_string(), "0");
    }

    #[test]
    fn fmt_num_single_digit() {
        assert_eq!(Sep(1).to_string(), "1");
    }

    #[test]
    fn fmt_num_three_digits() {
        assert_eq!(Sep(999).to_string(), "999");
    }

    #[test]
    fn fmt_num_four_digits() {
        assert_eq!(Sep(1_000).to_string(), "1,000");
    }

    #[test]
    fn fmt_num_mixed() {
        assert_eq!(Sep(1_234).to_string(), "1,234");
    }

    #[test]
    fn fmt_num_large() {
        assert_eq!(Sep(1_234_567).to_string(), "1,234,567");
    }

    #[test]
    fn fmt_num_millions_even() {
        assert_eq!(Sep(1_000_000).to_string(), "1,000,000");
    }

    #[test]
    fn fmt_date_epoch() {
        assert_eq!(fmt_date(0, "%Y-%m-%d"), "1970-01-01");
    }

    #[test]
    fn fmt_date_known_year() {
        assert_eq!(fmt_date(1_000_000_000, "%Y"), "2001");
    }
}
