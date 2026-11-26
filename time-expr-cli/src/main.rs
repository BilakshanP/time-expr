mod cli;

use std::process::ExitCode;

use clap::Parser;

use time_expr::prelude::*;

use cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();

    let source = match cli.source() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let input = source.text.trim();

    // Parse with span-rich errors so we can render diagnostics on failure.
    let expr = match parse_rich(input) {
        Ok(expr) => expr,
        Err(errs) => {
            report(input, &source.name, errs, &mut std::io::stderr())
                .expect("write diagnostics to stderr");
            return ExitCode::FAILURE;
        }
    };

    // Evaluate; type errors / division by zero surface here.
    let value = match expr.eval() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if cli.raw {
        print!("{value:?}");
    } else {
        print!("{value}");
    }

    if !cli.no_newline {
        println!();
    }

    ExitCode::SUCCESS
}
