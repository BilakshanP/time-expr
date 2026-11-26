# time-expr

An affine time-algebra library and a small expression language for it.

`time-expr` models time with two distinct types and a type-checked set of
operations between them:

- **TimePoint** — an absolute instant relative to Day 0, 00:00:00 (e.g. `14:30`).
- **Duration** — a signed displacement (e.g. `2h15m`, `-45m`).
- **Scalar** — a dimensionless number used to scale durations.

The algebra enforces what is meaningful. Subtracting two instants gives a
displacement; adding a displacement to an instant gives another instant; but
adding two instants is a type error, caught before evaluation.

## Workspace layout

| Crate           | What it is                                             |
| --------------- | ------------------------------------------------------ |
| `time-expr`     | The library: types, algebra, AST, parser, diagnostics. |
| `time-expr-cli` | The `texpr` command-line tool.                         |

## The type algebra

Rows are the left operand, columns the right. Each cell shows the allowed
operators and the result type (`t` = TimePoint, `d` = Duration, `s` = Scalar;
`na` = no legal operation).

```
l\r   T          D               S
T     (-)d       (+ -)t          na
D     (+)t       (+ -)d, (/)s     (* /)d
S     na         (*)d            (+ - * /)s
```

Notable rules:

- `TimePoint - TimePoint = Duration`; `TimePoint + TimePoint` is a type error.
- `Duration + TimePoint` is allowed (commutes with `TimePoint + Duration`), but
  `Duration - TimePoint` is not — you cannot subtract an instant from a
  displacement.
- `Duration / Duration = Scalar` (a ratio); there is no `Duration * Duration`.
- `TimePoint` never combines with `Scalar`.
- Dividing by a zero `Scalar` or a zero-length `Duration` is an error.

## Literal syntax

| Kind      | Form                                   | Examples                          |
| --------- | -------------------------------------- | --------------------------------- |
| TimePoint | `HH:MM`, `HH:MM:SS`, `DD:HH:MM:SS`     | `08:00`, `14:30:15`, `1:14:30:00` |
| Duration  | `[-]<n>d<n>h<n>m<n>s` (parts optional) | `2h15m`, `1d4s`, `-45m`, `0s`     |
| Scalar    | a plain number                         | `3`, `2.5`                        |

TimePoint fields are positional and distinguished by count: 2 fields = `HH:MM`,
3 = `HH:MM:SS`, 4 = `DD:HH:MM:SS`. The day may be negative; stepping back across
midnight wraps into the previous day (`00:00 - 1h` is `-1:23:00:00`, read as
"−1 day + 23h").

## Operators and precedence

`*` and `/` bind tighter than `+` and `-`; all are left-associative. Round
parentheses `( )` group sub-expressions. Whitespace is insignificant.

## CLI usage

```sh
# Install from the workspace
cargo install --path time-expr-cli

time-expr '(17:48 - 10:10) + (16:41 - 08:19)'   # => 16h
time-expr '2h15m + 45m'                          # => 3h
time-expr '6h / 2h'                              # => 3   (ratio, a Scalar)
echo '23:00 + 3h' | time-expr --stdin            # => 1:02:00:00
```

Flags: `--stdin` / `--file <FILE>` to choose input, `--raw` for the debug form,
`--no-newline` to omit the trailing newline, `--max-size <BYTES>` to cap input.

Parse errors are rendered with source-spanned diagnostics:

```
$ time-expr '2h +'
Error: empty duration
   ╭─[ <arg>:1:5 ]
   │
 1 │ 2h +
   │     │
   │     ╰─ empty duration
───╯
```

## Library usage

```rust
use time_expr::prelude::*;

let value = parse("(17:48 - 10:10) + (16:41 - 08:19)")?
    .eval()?;

assert_eq!(value.to_string(), "16h");
# Ok::<(), time_expr::error::TimeExprError>(())
```

- `parse(&str) -> Result<Expr, ParseError>` — `ParseError` collects the parser's
  reported problems as messages (today the parser stops at the first, so this is
  usually one; the type is a `Vec` so richer reporting can grow in later).
- `parse_rich(&str) -> Result<Expr, Vec<Rich<char>>>` — keeps spans for rich
  reporting; feed the errors to `diagnostics::report` (feature `diagnostics`).
- `Expr::eval(&self) -> Result<Value, EvalError>` — evaluates the tree,
  propagating the first type/arithmetic error.
- `TimeExprError` unifies `ParseError` and `EvalError` behind one `?`.

## Features

- `diagnostics` (on `time-expr`): enables `ariadne`-backend
  `report()` for spanned error output. The CLI enables it by default.

## License

MIT.
