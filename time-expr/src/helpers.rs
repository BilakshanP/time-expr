//! Time-unit constants and conversion helpers.
//!
//! These are internal building blocks shared by [`value`](crate::value) and
//! [`parser`](crate::parser). The constants define the fixed ratios between
//! days, hours, minutes and seconds; the functions convert between a flat
//! second count and the `(day, hours, mins, secs)` decomposition.

// =========================================================================
// Constants
// =========================================================================

pub const HOURS_PER_DAY: u32 = 24;
pub const MINUTES_PER_HOUR: u32 = 60;
pub const SECONDS_PER_MINUTE: u32 = 60;
pub const SECONDS_PER_HOUR: u32 = MINUTES_PER_HOUR * SECONDS_PER_MINUTE;
pub const SECONDS_PER_DAY: i64 = (HOURS_PER_DAY * SECONDS_PER_HOUR) as i64;

// =========================================================================
// Conversions
// =========================================================================

#[inline]
pub fn dhms_to_seconds(day: i64, hours: u32, mins: u32, secs: u32) -> i64 {
    let day_secs = day * SECONDS_PER_DAY;
    let time_secs = (hours * SECONDS_PER_HOUR + mins * SECONDS_PER_MINUTE + secs) as i64;

    day_secs + time_secs
}

/// Overflow-safe variant of [`dhms_to_seconds`] for signed component values.
///
/// All components are `i64` here (unlike [`dhms_to_seconds`], which takes the
/// already-validated `u32` clock fields) because the parser feeds raw,
/// possibly huge magnitudes straight from source text. Returns `None` if any
/// multiplication or addition would overflow `i64`, so callers can surface a
/// clean error instead of panicking or silently wrapping.
#[inline]
pub fn checked_dhms_to_seconds(day: i64, hours: i64, mins: i64, secs: i64) -> Option<i64> {
    day.checked_mul(SECONDS_PER_DAY)?
        .checked_add(hours.checked_mul(SECONDS_PER_HOUR as i64)?)?
        .checked_add(mins.checked_mul(SECONDS_PER_MINUTE as i64)?)?
        .checked_add(secs)
}

#[inline]
pub fn extract_day_component(secs: i64) -> i64 {
    secs.div_euclid(SECONDS_PER_DAY)
}

#[inline]
pub fn extract_hms_components(secs: i64) -> (u32, u32, u32) {
    let hms_secs = secs.rem_euclid(SECONDS_PER_DAY) as u32;

    (
        hms_secs / SECONDS_PER_HOUR,
        (hms_secs % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE,
        hms_secs % SECONDS_PER_MINUTE,
    )
}

#[inline]
pub fn extract_dhms_components(secs: i64) -> (i64, u32, u32, u32) {
    let d = extract_day_component(secs);
    let (h, m, s) = extract_hms_components(secs);

    (d, h, m, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    // === constants ========================================================

    #[test]
    fn constant_values() {
        assert_eq!(HOURS_PER_DAY * MINUTES_PER_HOUR, 1_440);
        assert_eq!(SECONDS_PER_HOUR, 3_600);
        assert_eq!(SECONDS_PER_DAY, 86_400);
    }

    // === dhms_to_seconds =================================================

    #[test]
    fn test_dhms_to_seconds() {
        assert_eq!(dhms_to_seconds(0, 0, 0, 0), 0);
        assert_eq!(dhms_to_seconds(0, 0, 0, 1), 1);
        assert_eq!(dhms_to_seconds(0, 0, 0, 59), 59);
        assert_eq!(dhms_to_seconds(0, 0, 0, 60), 60);

        assert_eq!(dhms_to_seconds(0, 0, 1, 0), SECONDS_PER_MINUTE as i64);
        assert_eq!(dhms_to_seconds(0, 1, 0, 0), SECONDS_PER_HOUR as i64);
        assert_eq!(dhms_to_seconds(1, 0, 0, 0), SECONDS_PER_DAY);

        // Carrying one unit's worth equals one of the larger unit.
        assert_eq!(
            dhms_to_seconds(0, 0, 0, SECONDS_PER_MINUTE),
            dhms_to_seconds(0, 0, 1, 0),
        );
        assert_eq!(
            dhms_to_seconds(0, 0, MINUTES_PER_HOUR, 0),
            dhms_to_seconds(0, 1, 0, 0),
        );
        assert_eq!(
            dhms_to_seconds(0, HOURS_PER_DAY, 0, 0),
            dhms_to_seconds(1, 0, 0, 0),
        );

        assert_eq!(
            dhms_to_seconds(1, 1, 1, 0),
            SECONDS_PER_DAY + SECONDS_PER_HOUR as i64 + SECONDS_PER_MINUTE as i64
        );
        assert_eq!(dhms_to_seconds(10, 20, 30, 40), 937_840);

        // Negative days.
        assert_eq!(dhms_to_seconds(-1, 0, 0, 0), -SECONDS_PER_DAY);
        assert_eq!(
            dhms_to_seconds(-1, 1, 0, 0),
            -SECONDS_PER_DAY + SECONDS_PER_HOUR as i64
        );

        // Additivity.
        assert_eq!(
            dhms_to_seconds(1, 1, 1, 1),
            dhms_to_seconds(1, 0, 0, 0)
                + dhms_to_seconds(0, 1, 0, 0)
                + dhms_to_seconds(0, 0, 1, 0)
                + dhms_to_seconds(0, 0, 0, 1)
        );
        assert_eq!(
            dhms_to_seconds(-1, 1, 0, 0),
            dhms_to_seconds(-1, 0, 0, 0) + dhms_to_seconds(0, 1, 0, 0)
        );
    }

    // === extract_day_component ===========================================

    #[test]
    fn test_extract_day_component() {
        fn get_day(d: i64, h: u32, m: u32, s: u32) -> i64 {
            extract_day_component(dhms_to_seconds(d, h, m, s))
        }

        assert_eq!(get_day(0, 0, 0, 0), 0);
        assert_eq!(get_day(1, 0, 0, 0), 1);
        assert_eq!(get_day(999, 0, 0, 0), 999);
        assert_eq!(get_day(-1, 0, 0, 0), -1);
        assert_eq!(get_day(-998, 0, 0, 0), -998);

        assert_eq!(get_day(0, HOURS_PER_DAY, 0, 0), 1);
        assert_eq!(
            get_day(0, 0, HOURS_PER_DAY * MINUTES_PER_HOUR, 0),
            1
        );
        assert_eq!(get_day(0, 0, 0, SECONDS_PER_DAY as u32), 1);
    }

    // === extract_hms_components ==========================================

    #[test]
    fn test_extract_hms_components() {
        fn get_hms(d: i64, h: u32, m: u32, s: u32) -> (u32, u32, u32) {
            extract_hms_components(dhms_to_seconds(d, h, m, s))
        }

        assert_eq!(get_hms(0, 0, 0, 0), (0, 0, 0));
        assert_eq!(get_hms(0, 14, 30, 45), (14, 30, 45));
        assert_eq!(get_hms(0, 23, 59, 59), (23, 59, 59));
        assert_eq!(get_hms(5, 14, 30, 45), (14, 30, 45));
        assert_eq!(get_hms(0, HOURS_PER_DAY, 0, 0), (0, 0, 0));

        // Euclidean wrap for negatives.
        assert_eq!(extract_hms_components(-1), (23, 59, 59));
        assert_eq!(
            extract_hms_components(-(SECONDS_PER_HOUR as i64)),
            (23, 0, 0)
        );
    }

    // === extract_dhms_components =========================================

    #[test]
    fn test_extract_dhms_components() {
        fn get_dhms(d: i64, h: u32, m: u32, s: u32) -> (i64, u32, u32, u32) {
            extract_dhms_components(dhms_to_seconds(d, h, m, s))
        }

        assert_eq!(get_dhms(0, 0, 0, 0), (0, 0, 0, 0));
        assert_eq!(get_dhms(1, 14, 30, 45), (1, 14, 30, 45));
        assert_eq!(get_dhms(0, HOURS_PER_DAY, 0, 0), (1, 0, 0, 0));

        // Negative wrapping.
        assert_eq!(extract_dhms_components(-1), (-1, 23, 59, 59));
        assert_eq!(
            extract_dhms_components(-(SECONDS_PER_HOUR as i64)),
            (-1, 23, 0, 0)
        );
        assert_eq!(
            extract_dhms_components(dhms_to_seconds(-1, 0, 0, 0)),
            (-1, 0, 0, 0)
        );

        // Consistency: dhms == (day_component, hms_components).
        let secs = dhms_to_seconds(7, 8, 9, 10);
        let (d, h, m, s) = extract_dhms_components(secs);
        assert_eq!(d, extract_day_component(secs));
        assert_eq!((h, m, s), extract_hms_components(secs));
    }

    // === checked_dhms ====================================================

    #[test]
    fn checked_dhms_matches_unchecked_when_in_range() {
        assert_eq!(
            checked_dhms_to_seconds(1, 2, 3, 4),
            Some(dhms_to_seconds(1, 2, 3, 4))
        );
        assert_eq!(checked_dhms_to_seconds(0, 0, 0, 0), Some(0));
        assert_eq!(
            checked_dhms_to_seconds(-1, 1, 0, 0),
            Some(-SECONDS_PER_DAY + SECONDS_PER_HOUR as i64)
        );
    }

    #[test]
    fn checked_dhms_returns_none_on_overflow() {
        assert_eq!(checked_dhms_to_seconds(i64::MAX, 0, 0, 0), None);
        assert_eq!(
            checked_dhms_to_seconds(i64::MAX / 86_400, i64::MAX, 0, 0),
            None
        );
    }
}
