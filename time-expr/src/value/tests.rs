use super::*;
use crate::{error::EvalError, op::BinOp};

// === Duration display ====================================================

#[test]
fn duration_display_omits_zero_components() {
    assert_eq!(Duration::from_dhms(1, 2, 3, 4).to_string(), "1d2h3m4s");
    assert_eq!(Duration::from_dhms(1, 0, 0, 4).to_string(), "1d4s");
    assert_eq!(Duration::from_dhms(0, 5, 0, 0).to_string(), "5h");
}

#[test]
fn duration_display_zero_is_0s() {
    assert_eq!(Duration::from_secs(0).to_string(), "0s");
}

#[test]
fn duration_display_negative_has_single_leading_sign() {
    assert_eq!(Duration::from_dhms(0, 2, 30, 0).to_string(), "2h30m");
    assert_eq!(
        Duration::from_secs(-(2 * 3600 + 30 * 60)).to_string(),
        "-2h30m"
    );
    assert_eq!(Duration::from_dhms(-1, 0, 0, 0).to_string(), "-1d");
}

#[test]
fn duration_display_i64_min_does_not_panic() {
    // i64::MIN's magnitude (2^63) doesn't fit in i64. The Display impl must
    // handle this without overflow or panic.
    let d = Duration::from_secs(i64::MIN);
    let s = d.to_string();
    assert!(s.starts_with('-'), "should be negative: {s}");
    assert!(!s.is_empty());
}

// === TimePoint display ===================================================

#[test]
fn timepoint_display_day0_whole_minute_is_two_fields() {
    let tp = TimePoint::from_dhms_strict(0, 8, 0, 0).unwrap();
    assert_eq!(tp.to_string(), "08:00");
}

#[test]
fn timepoint_display_day0_with_seconds_is_three_fields() {
    let tp = TimePoint::from_dhms_strict(0, 8, 0, 30).unwrap();
    assert_eq!(tp.to_string(), "08:00:30");
}

#[test]
fn timepoint_display_nonzero_day_is_four_fields() {
    let tp = TimePoint::from_dhms_strict(1, 14, 30, 0).unwrap();
    assert_eq!(tp.to_string(), "1:14:30:00");
}

#[test]
fn timepoint_display_negative_day_wraps_and_signs() {
    let tp = TimePoint::from_secs(0) - Duration::from_dhms(0, 1, 0, 0);
    assert_eq!(tp.to_string(), "-1:23:00:00");
}

// === Value: add ==========================================================

#[test]
fn add_scalar_scalar_is_scalar() {
    let r = Value::Scalar(2.0).add(Value::Scalar(5.0)).unwrap();
    assert_eq!(r, Value::Scalar(7.0));
}

#[test]
fn add_timepoint_duration_is_timepoint() {
    let r = tp(0, 1, 0, 0).add(dur(0, 2, 0, 0)).unwrap();
    assert_eq!(as_timepoint_secs(r), 3 * 3600);
}

#[test]
fn add_duration_timepoint_is_timepoint_commutative() {
    let r = dur(0, 2, 0, 0).add(tp(0, 1, 0, 0)).unwrap();
    assert_eq!(as_timepoint_secs(r), 3 * 3600);
}

#[test]
fn add_duration_duration_is_duration() {
    let r = dur(0, 2, 0, 0).add(dur(0, 3, 0, 0)).unwrap();
    assert_eq!(as_duration_secs(r), 5 * 3600);
}

// === Value: sub ==========================================================

#[test]
fn sub_scalar_scalar_is_scalar() {
    let r = Value::Scalar(10.0).sub(Value::Scalar(4.0)).unwrap();
    assert_eq!(r, Value::Scalar(6.0));
}

#[test]
fn sub_timepoint_timepoint_is_duration() {
    let r = tp(0, 5, 0, 0).sub(tp(0, 2, 0, 0)).unwrap();
    assert_eq!(as_duration_secs(r), 3 * 3600);
}

#[test]
fn sub_timepoint_duration_is_timepoint() {
    let r = tp(0, 5, 0, 0).sub(dur(0, 2, 0, 0)).unwrap();
    assert_eq!(as_timepoint_secs(r), 3 * 3600);
}

#[test]
fn sub_duration_duration_is_duration() {
    let r = dur(0, 5, 0, 0).sub(dur(0, 2, 0, 0)).unwrap();
    assert_eq!(as_duration_secs(r), 3 * 3600);
}

// === Value: mul ==========================================================

#[test]
fn mul_scalar_scalar_is_scalar() {
    let r = Value::Scalar(3.0).mul(Value::Scalar(4.0)).unwrap();
    assert_eq!(r, Value::Scalar(12.0));
}

#[test]
fn mul_duration_scalar_is_duration() {
    let r = dur(0, 2, 0, 0).mul(Value::Scalar(3.0)).unwrap();
    assert_eq!(as_duration_secs(r), 6 * 3600);
}

#[test]
fn mul_scalar_duration_is_duration_commutative() {
    let r = Value::Scalar(3.0).mul(dur(0, 2, 0, 0)).unwrap();
    assert_eq!(as_duration_secs(r), 6 * 3600);
}

// === Value: div ==========================================================

#[test]
fn div_scalar_scalar_is_scalar() {
    let r = Value::Scalar(6.0).div(Value::Scalar(2.0)).unwrap();
    assert_eq!(r, Value::Scalar(3.0));
}

#[test]
fn div_is_division_not_addition_regression_guard() {
    assert_eq!(
        Value::Scalar(6.0).div(Value::Scalar(2.0)).unwrap(),
        Value::Scalar(3.0)
    );
}

#[test]
fn div_duration_duration_is_scalar_ratio() {
    let r = dur(0, 6, 0, 0).div(dur(0, 2, 0, 0)).unwrap();
    assert_eq!(r, Value::Scalar(3.0));
}

#[test]
fn div_duration_scalar_is_duration() {
    let r = dur(0, 6, 0, 0).div(Value::Scalar(3.0)).unwrap();
    assert_eq!(as_duration_secs(r), 2 * 3600);
}

// === Value: division by zero =============================================

#[test]
fn div_duration_by_zero_scalar_errors() {
    assert_eq!(
        dur(0, 1, 0, 0).div(Value::Scalar(0.0)),
        Err(EvalError::DivisionByZero)
    );
}

#[test]
fn div_duration_by_zero_duration_errors() {
    assert_eq!(
        dur(0, 1, 0, 0).div(dur_secs(0)),
        Err(EvalError::DivisionByZero)
    );
}

#[test]
fn div_scalar_by_zero_scalar_errors() {
    assert_eq!(
        Value::Scalar(5.0).div(Value::Scalar(0.0)),
        Err(EvalError::DivisionByZero)
    );
}

#[test]
fn div_scalar_by_zero_duration_errors() {
    assert_eq!(
        Value::Scalar(5.0).div(dur_secs(0)),
        Err(EvalError::DivisionByZero)
    );
}

// === Value: invalid operations ===========================================

#[test]
fn add_timepoint_timepoint_is_invalid() {
    assert_eq!(
        tp(0, 1, 0, 0).add(tp(0, 2, 0, 0)),
        Err(EvalError::InvalidOperation {
            op: BinOp::Add,
            lhs: "TimePoint",
            rhs: "TimePoint",
        })
    );
}

#[test]
fn add_scalar_duration_is_invalid() {
    assert_eq!(
        Value::Scalar(1.0).add(dur(0, 1, 0, 0)),
        Err(EvalError::InvalidOperation {
            op: BinOp::Add,
            lhs: "Scalar",
            rhs: "Duration",
        })
    );
}

#[test]
fn add_scalar_timepoint_is_invalid() {
    assert_eq!(
        Value::Scalar(1.0).add(tp(0, 1, 0, 0)),
        Err(EvalError::InvalidOperation {
            op: BinOp::Add,
            lhs: "Scalar",
            rhs: "TimePoint",
        })
    );
}

#[test]
fn sub_duration_timepoint_is_invalid() {
    assert_eq!(
        dur(0, 1, 0, 0).sub(tp(0, 1, 0, 0)),
        Err(EvalError::InvalidOperation {
            op: BinOp::Sub,
            lhs: "Duration",
            rhs: "TimePoint",
        })
    );
}

#[test]
fn sub_scalar_timepoint_is_invalid() {
    assert_eq!(
        Value::Scalar(1.0).sub(tp(0, 1, 0, 0)),
        Err(EvalError::InvalidOperation {
            op: BinOp::Sub,
            lhs: "Scalar",
            rhs: "TimePoint",
        })
    );
}

#[test]
fn mul_timepoint_timepoint_is_invalid() {
    assert_eq!(
        tp(0, 1, 0, 0).mul(tp(0, 2, 0, 0)),
        Err(EvalError::InvalidOperation {
            op: BinOp::Mul,
            lhs: "TimePoint",
            rhs: "TimePoint",
        })
    );
}

#[test]
fn mul_timepoint_duration_is_invalid() {
    assert_eq!(
        tp(0, 1, 0, 0).mul(dur(0, 1, 0, 0)),
        Err(EvalError::InvalidOperation {
            op: BinOp::Mul,
            lhs: "TimePoint",
            rhs: "Duration",
        })
    );
}

#[test]
fn mul_timepoint_scalar_is_invalid() {
    assert_eq!(
        tp(0, 1, 0, 0).mul(Value::Scalar(2.0)),
        Err(EvalError::InvalidOperation {
            op: BinOp::Mul,
            lhs: "TimePoint",
            rhs: "Scalar",
        })
    );
}

#[test]
fn div_timepoint_duration_is_invalid() {
    assert_eq!(
        tp(0, 1, 0, 0).div(dur(0, 1, 0, 0)),
        Err(EvalError::InvalidOperation {
            op: BinOp::Div,
            lhs: "TimePoint",
            rhs: "Duration",
        })
    );
}

#[test]
fn div_timepoint_scalar_is_invalid() {
    assert_eq!(
        tp(0, 1, 0, 0).div(Value::Scalar(2.0)),
        Err(EvalError::InvalidOperation {
            op: BinOp::Div,
            lhs: "TimePoint",
            rhs: "Scalar",
        })
    );
}

#[test]
fn div_scalar_timepoint_is_invalid() {
    assert_eq!(
        Value::Scalar(2.0).div(tp(0, 1, 0, 0)),
        Err(EvalError::InvalidOperation {
            op: BinOp::Div,
            lhs: "Scalar",
            rhs: "TimePoint",
        })
    );
}

// === test helpers ========================================================

fn dur(day: i64, hours: u32, mins: u32, secs: u32) -> Value {
    Value::Duration(Duration::from_dhms(day, hours, mins, secs))
}

fn dur_secs(secs: i64) -> Value {
    Value::Duration(Duration::from_secs(secs))
}

fn tp(day: i64, hours: u32, mins: u32, secs: u32) -> Value {
    Value::TimePoint(TimePoint::from_dhms_strict(day, hours, mins, secs).unwrap())
}

fn as_duration_secs(v: Value) -> i64 {
    match v {
        Value::Duration(d) => d.secs(),
        other => panic!("expected Duration, got {other:?}"),
    }
}

fn as_timepoint_secs(v: Value) -> i64 {
    match v {
        Value::TimePoint(t) => t.secs(),
        other => panic!("expected TimePoint, got {other:?}"),
    }
}
