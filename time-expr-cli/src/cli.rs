use std::io::Read;
use std::path::PathBuf;

use clap::{Args, Parser};

const AFTER_HELP: &str = "\
LITERALS:
  TimePoint   HH:MM | HH:MM:SS | DD:HH:MM:SS         e.g. 08:00, 14:30:15, 1:14:30:00
  Duration    [-]<n>d<n>h<n>m<n>s (parts optional)   e.g. 2h15m, 1d4s, -45m, 0s
  Scalar      a plain number                         e.g. 3, 2.5

OPERATORS (precedence high to low):
  ( )         grouping — overrides all precedence
  unary -     Negate a Duration or Scalar (e.g. -2h, -(5h - 2h))
  *, /        Duration*Scalar, Scalar*Duration, Duration/Scalar, Duration/Duration (ratio)
  +, -        TimePoint-TimePoint=Duration, TimePoint+/-Duration, Duration+/-Duration

EXAMPLES:
  texpr '(17:48 - 10:10) + (16:41 - 08:19)'    # => 16h
  texpr '2h15m + 45m'                           # => 3h
  texpr '6h / 2h'                               # => 3  (ratio, a Scalar)
  echo '23:00 + 3h' | texpr --stdin             # => 1:02:00:00
";

/// time-expr: evaluate affine time expressions.
///
/// Combines TimePoints (clock instants), Durations (signed displacements), and
/// Scalars under a type-checked algebra: e.g. `TimePoint - TimePoint` yields a
/// Duration, but `TimePoint + TimePoint` is rejected.
#[derive(Parser)]
#[command(
    name = "time-expr",
    version,
    about = "Evaluate affine time expressions (TimePoints, Durations, Scalars).",
    long_about = None,
    after_help = AFTER_HELP,
)]
pub struct Cli {
    #[command(flatten)]
    pub input: Input,

    /// Emit the raw debug representation of the result instead of its display form.
    #[arg(short, long)]
    pub raw: bool,

    /// Do not print a trailing newline.
    #[arg(short = 'n', long = "no-newline")]
    pub no_newline: bool,

    /// Maximum input size in bytes (default: 1 MiB).
    #[arg(long, default_value = "1048576", value_name = "BYTES")]
    pub max_size: usize,
}

/// A resolved input: its display name (for diagnostics) and the source text.
pub struct Source {
    pub name: String,
    pub text: String,
}

impl Source {
    fn new(name: impl Into<String>, text: String) -> Self {
        Source {
            name: name.into(),
            text,
        }
    }
}

#[derive(Args)]
#[group(required = true, multiple = false)]
pub struct Input {
    /// The expression to evaluate, e.g. "(17:48 - 10:10) + (16:41 - 08:19)".
    ///
    /// Expressions starting with '-' (e.g. "-24h") are accepted without '--'.
    #[arg(allow_hyphen_values = true)]
    pub expr: Option<String>,

    /// Read the expression from stdin.
    #[arg(short, long)]
    pub stdin: bool,

    /// Read the expression from a file.
    #[arg(short, long, value_name = "FILE")]
    pub file: Option<PathBuf>,
}

impl Cli {
    /// Resolve the chosen input source into its text, enforcing `max_size`.
    pub fn source(&self) -> Result<Source, String> {
        let src = self.input.read()?;

        if src.text.len() > self.max_size {
            return Err(format!(
                "input exceeds maximum size ({} bytes > {} bytes limit)",
                src.text.len(),
                self.max_size
            ));
        }

        Ok(src)
    }
}

impl Input {
    fn read(&self) -> Result<Source, String> {
        if let Some(expr) = &self.expr {
            return Ok(Source::new("<arg>", expr.clone()));
        }

        if let Some(path) = &self.file {
            let name = path.display().to_string();
            let text = std::fs::read_to_string(path).map_err(|e| format!("{name}: {e}"))?;
            return Ok(Source::new(name, text));
        }

        if self.stdin {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| format!("reading stdin: {e}"))?;
            return Ok(Source::new("<stdin>", text));
        }

        Err("one of <EXPR>, --file, or --stdin must be provided".to_string())
    }
}
