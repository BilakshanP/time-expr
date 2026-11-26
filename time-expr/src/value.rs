//! Core value types and the dynamic dispatch layer for the time algebra.
//!
//! Three domain types model the time algebra:
//! - [`Duration`] — a signed displacement (e.g. `2h15m`, `-45m`).
//! - [`TimePoint`] — an absolute instant relative to Day 0, 00:00:00.
//! - [`Scalar`] — a dimensionless number (`f64`) used to scale durations.
//!
//! [`Value`] wraps all three behind a single enum so the evaluator can work
//! with dynamically-typed operands while enforcing the affine type rules at
//! runtime.

use std::{fmt, ops};

use crate::{
    error::{EvalError, TimeError},
    helpers::{
        dhms_to_seconds, extract_day_component, extract_dhms_components, extract_hms_components,
        HOURS_PER_DAY, MINUTES_PER_HOUR, SECONDS_PER_MINUTE,
    },
    op::{BinOp, UnaryOp},
};

// =========================================================================
// 1. Scalar
// =========================================================================

/// A dimensionless number used to scale durations.
pub type Scalar = f64;

// =========================================================================
// 2. Duration
// =========================================================================

/// A signed displacement in time, stored as a count of seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Duration(i64);

impl Duration {
    #[inline]
    pub fn from_secs(secs: i64) -> Self {
        Self(secs)
    }

    /// Build from day/hours/mins/secs. No range checks — components simply sum
    /// into total seconds.
    #[inline]
    pub fn from_dhms(day: i64, hours: u32, mins: u32, secs: u32) -> Self {
        Self::from_secs(dhms_to_seconds(day, hours, mins, secs))
    }

    #[inline]
    pub fn secs(&self) -> i64 {
        self.0
    }

    #[inline]
    pub fn days(self) -> i64 {
        extract_day_component(self.secs())
    }

    pub fn hms(self) -> (u32, u32, u32) {
        extract_hms_components(self.secs())
    }

    pub fn dhms(self) -> (i64, u32, u32, u32) {
        let (h, m, s) = self.hms();
        (self.days(), h, m, s)
    }
}

impl fmt::Display for Duration {
    /// Suffix form that the parser round-trips: `1d2h3m4s`, with any zero
    /// component omitted (`1d4s` is valid). A whole-magnitude negative prints
    /// a single leading `-` (`-2h30m`). The all-zero duration prints `0s`,
    /// since an empty string would not round-trip.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return write!(f, "0s");
        }

        if self.0 < 0 {
            write!(f, "-")?;
        }

        // Work with the unsigned magnitude so we never hit the i64::MIN
        // overflow that `.abs()` would cause. `unsigned_abs()` returns u64;
        // we keep it as u64 through the decomposition to stay correct even
        // for `i64::MIN` (whose magnitude, 2^63, doesn't fit in i64).
        let mag = self.0.unsigned_abs();
        let spd = crate::helpers::SECONDS_PER_DAY as u64;
        let sph = crate::helpers::SECONDS_PER_HOUR as u64;
        let spm = crate::helpers::SECONDS_PER_MINUTE as u64;

        let d = mag / spd;
        let rem = mag % spd;
        let h = rem / sph;
        let rem = rem % sph;
        let m = rem / spm;
        let s = rem % spm;

        if d != 0 {
            write!(f, "{d}d")?;
        }
        if h != 0 {
            write!(f, "{h}h")?;
        }
        if m != 0 {
            write!(f, "{m}m")?;
        }
        if s != 0 {
            write!(f, "{s}s")?;
        }

        Ok(())
    }
}

// =========================================================================
// 3. TimePoint
// =========================================================================

/// An absolute instant relative to Day 0, 00:00, stored as total seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimePoint(i64);

impl TimePoint {
    #[inline]
    pub fn from_secs(secs: i64) -> Self {
        Self(secs)
    }

    /// Canonical constructor: rejects out-of-range time components
    /// (`hours >= 24`, `mins >= 60`, `secs >= 60`) instead of normalizing
    /// them. The `day` may be any value, including negative. Use
    /// [`from_dhms_normalized`] to accept and normalize overflowing
    /// components instead.
    ///
    /// [`from_dhms_normalized`]: Self::from_dhms_normalized
    pub fn from_dhms_strict(day: i64, hours: u32, mins: u32, secs: u32) -> Result<Self, TimeError> {
        if hours >= HOURS_PER_DAY || mins >= MINUTES_PER_HOUR || secs >= SECONDS_PER_MINUTE {
            return Err(TimeError {
                day,
                hours,
                mins,
                secs,
            });
        }

        Ok(Self::from_dhms_normalized(day, hours, mins, secs))
    }

    /// Lenient constructor: accepts any component values and normalizes them.
    /// Components simply sum into total seconds, so overflow carries upward
    /// (e.g. `25` hours becomes day 1, 01:00 on readback). Never fails.
    pub fn from_dhms_normalized(day: i64, hours: u32, mins: u32, secs: u32) -> Self {
        Self::from_secs(dhms_to_seconds(day, hours, mins, secs))
    }

    #[inline]
    pub fn secs(self) -> i64 {
        self.0
    }

    #[inline]
    pub fn days(self) -> i64 {
        extract_day_component(self.secs())
    }

    pub fn hms(self) -> (u32, u32, u32) {
        extract_hms_components(self.secs())
    }

    pub fn dhms(self) -> (i64, u32, u32, u32) {
        extract_dhms_components(self.secs())
    }
}

impl fmt::Display for TimePoint {
    /// Positional clock form that the parser round-trips, in the shortest
    /// unambiguous shape:
    /// - `HH:MM` on day 0 with zero seconds (e.g. `08:00`)
    /// - `HH:MM:SS` on day 0 with nonzero seconds (e.g. `08:00:30`)
    /// - `DD:HH:MM:SS` whenever the day is nonzero (e.g. `1:14:30:00`)
    ///
    /// H/M/S are always zero-padded to two digits; the day is printed as-is
    /// (it may exceed two digits and may be negative, e.g. `-1:23:00:00`).
    /// The meaning is unambiguous by field count: 2 = HH:MM, 3 = HH:MM:SS,
    /// 4 = DD:HH:MM:SS. Seconds are forced whenever a day is present, since a
    /// 4-field form must carry all four fields.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (day, h, m, s) = self.dhms();

        if day != 0 {
            write!(f, "{day}:")?;
        }

        write!(f, "{h:02}:{m:02}")?;

        if s != 0 || day != 0 {
            write!(f, ":{s:02}")?;
        }

        Ok(())
    }
}

// =========================================================================
// 4. Affine operator impls (std::ops on the concrete types)
// =========================================================================

/// TimePoint - TimePoint = Duration
impl ops::Sub for TimePoint {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        Duration(self.0 - rhs.0)
    }
}

/// TimePoint - Duration = TimePoint
impl ops::Sub<Duration> for TimePoint {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self::Output {
        TimePoint(self.0 - rhs.0)
    }
}

/// TimePoint + Duration = TimePoint
impl ops::Add<Duration> for TimePoint {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        TimePoint(self.0 + rhs.0)
    }
}

/// Duration + TimePoint = TimePoint
impl ops::Add<TimePoint> for Duration {
    type Output = TimePoint;

    fn add(self, rhs: TimePoint) -> Self::Output {
        rhs + self
    }
}

/// Duration + Duration = Duration
impl ops::Add for Duration {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        Duration(self.0 + rhs.0)
    }
}

/// Duration - Duration = Duration
impl ops::Sub for Duration {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self::Output {
        Duration(self.0 - rhs.0)
    }
}

/// Duration / Duration = Scalar
impl ops::Div for Duration {
    type Output = Scalar;

    fn div(self, rhs: Duration) -> Self::Output {
        self.0 as Scalar / rhs.0 as Scalar
    }
}

/// Duration / Scalar = Duration
impl ops::Div<Scalar> for Duration {
    type Output = Self;

    fn div(self, rhs: Scalar) -> Self::Output {
        Duration((self.0 as Scalar / rhs).round() as i64)
    }
}

/// Duration * Scalar = Duration
impl ops::Mul<Scalar> for Duration {
    type Output = Self;

    fn mul(self, rhs: Scalar) -> Self::Output {
        Duration((self.0 as Scalar * rhs).round() as i64)
    }
}

/// Scalar * Duration = Duration
impl ops::Mul<Duration> for Scalar {
    type Output = Duration;

    fn mul(self, rhs: Duration) -> Self::Output {
        rhs * self
    }
}

// =========================================================================
// 5. Value — the dynamic dispatch wrapper
// =========================================================================

/// A dynamically-typed time value: an absolute [`TimePoint`], a signed
/// [`Duration`] displacement, or a dimensionless [`Scalar`].
///
/// ```text
/// 1.  TimePoint - TimePoint = Duration
/// 2.  TimePoint - Duration  = TimePoint
/// 3.  TimePoint + Duration  = TimePoint
/// 4.  Duration  + TimePoint = TimePoint
/// 5.  Duration  + Duration  = Duration
/// 6.  Duration  - Duration  = Duration
/// 7.  Duration  / Duration  = Scalar
/// 8.  Duration  / Scalar    = Duration
/// 9.  Duration  * Scalar    = Duration
/// 10. Scalar    * Duration  = Duration
///
/// 11. Scalar    + Scalar    = Scalar
/// 12. Scalar    - Scalar    = Scalar
/// 13. Scalar    * Scalar    = Scalar
/// 14. Scalar    / Scalar    = Scalar
/// ```
///
/// Notable asymmetries, enforced by [`Value::add`], [`Value::sub`],
/// [`Value::mul`], and [`Value::div`]:
/// - `TimePoint - TimePoint` yields a `Duration`; `TimePoint + TimePoint` is a
///   type error.
/// - `Duration + TimePoint` is legal (commutes with `TimePoint + Duration`),
///   but `Duration - TimePoint` is **not** — subtracting a point from a
///   displacement is meaningless.
/// - `Duration / Duration` collapses two durations into a dimensionless
///   `Scalar` ratio; there is no `Duration * Duration`.
/// - `TimePoint` never combines with `Scalar`.
///
/// Division by a zero `Scalar` or zero-length `Duration` returns
/// [`EvalError::DivisionByZero`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Scalar(Scalar),
    Duration(Duration),
    TimePoint(TimePoint),
}

#[allow(clippy::should_implement_trait)]
impl Value {
    pub fn add(self, rhs: Value) -> Result<Value, EvalError> {
        use Value::*;
        match (self, rhs) {
            (Scalar(s1), Scalar(s2)) => Ok(Scalar(s1 + s2)),
            (TimePoint(t), Duration(d)) | (Duration(d), TimePoint(t)) => Ok(TimePoint(t + d)),
            (Duration(d1), Duration(d2)) => Ok(Duration(d1 + d2)),
            _ => Err(self.invalid_op(BinOp::Add, &rhs)),
        }
    }

    pub fn sub(self, rhs: Value) -> Result<Value, EvalError> {
        use Value::*;
        match (self, rhs) {
            (Scalar(s1), Scalar(s2)) => Ok(Scalar(s1 - s2)),
            (TimePoint(t1), TimePoint(t2)) => Ok(Duration(t1 - t2)),
            (TimePoint(t), Duration(d)) => Ok(TimePoint(t - d)),
            (Duration(d1), Duration(d2)) => Ok(Duration(d1 - d2)),
            _ => Err(self.invalid_op(BinOp::Sub, &rhs)),
        }
    }

    pub fn mul(self, rhs: Value) -> Result<Value, EvalError> {
        use Value::*;
        match (self, rhs) {
            (Scalar(s1), Scalar(s2)) => Ok(Scalar(s1 * s2)),
            (Duration(d), Scalar(s)) | (Scalar(s), Duration(d)) => Ok(Duration(d * s)),
            _ => Err(self.invalid_op(BinOp::Mul, &rhs)),
        }
    }

    pub fn div(self, rhs: Value) -> Result<Value, EvalError> {
        use Value::*;
        match (self, rhs) {
            (_, Scalar(0.0)) => Err(EvalError::DivisionByZero),
            (_, Duration(d)) if d.secs() == 0 => Err(EvalError::DivisionByZero),

            (Scalar(s1), Scalar(s2)) => Ok(Scalar(s1 / s2)),
            (Duration(d1), Duration(d2)) => Ok(Scalar(d1 / d2)),
            (Duration(d), Scalar(s)) => Ok(Duration(d / s)),
            _ => Err(self.invalid_op(BinOp::Div, &rhs)),
        }
    }

    /// Unary negation. Negates a `Duration` or `Scalar`; negating a
    /// `TimePoint` is a type error (an absolute instant has no negative).
    pub fn neg(self) -> Result<Value, EvalError> {
        use Value::*;
        match self {
            Scalar(s) => Ok(Scalar(-s)),
            Duration(d) => Ok(Duration(self::Duration::from_secs(
                d.secs().checked_neg().unwrap_or(i64::MAX),
            ))),
            TimePoint(_) => Err(EvalError::InvalidUnaryOperation {
                op: UnaryOp::Neg,
                operand: self.type_name(),
            }),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Value::TimePoint(_) => "TimePoint",
            Value::Duration(_) => "Duration",
            Value::Scalar(_) => "Scalar",
        }
    }

    fn invalid_op(&self, op: BinOp, rhs: &Value) -> EvalError {
        EvalError::InvalidOperation {
            op,
            lhs: self.type_name(),
            rhs: rhs.type_name(),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Scalar(s) => write!(f, "{s}"),
            Value::Duration(d) => write!(f, "{d}"),
            Value::TimePoint(t) => write!(f, "{t}"),
        }
    }
}

#[cfg(test)]
mod tests;
