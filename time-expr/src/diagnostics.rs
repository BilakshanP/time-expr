//! Rich parse-error diagnostics via [`ariadne`].
//!
//! Feature-gated behind `diagnostics`. Turns the span-carrying [`Rich`] errors
//! from [`parse_rich`](crate::parser::parse_rich) into human-readable reports
//! that underline the offending region of the source.

use std::io::Write;

use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::error::Rich;

/// Render parse-error diagnostics to a writer.
///
/// # Arguments
/// * `input`  - the source text that was parsed
/// * `file`   - the filename/label shown in the report header
/// * `errs`   - the errors returned by [`parse_rich`](crate::parser::parse_rich)
/// * `output` - where the formatted report is written
///
/// # Example
/// ```
/// use time_expr::parser::parse_rich;
/// use time_expr::diagnostics::report;
///
/// let input = "2h +";
/// if let Err(errs) = parse_rich(input) {
///     let mut buf = Vec::new();
///     report(input, "<input>", errs, &mut buf).unwrap();
///     assert!(!buf.is_empty());
/// }
/// ```
pub fn report<W: Write>(
    input: &str,
    file: &str,
    errs: Vec<Rich<'_, char>>,
    output: &mut W,
) -> std::io::Result<()> {
    for err in errs {
        let span = err.span().into_range();

        Report::build(ReportKind::Error, (file, span.clone()))
            .with_message(err.reason().to_string())
            .with_label(
                Label::new((file, span))
                    .with_message(err.reason().to_string())
                    .with_color(Color::Red),
            )
            .finish()
            .write((file, Source::from(input)), &mut *output)?;
    }

    Ok(())
}
