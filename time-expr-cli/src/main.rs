mod cli;

use std::process::ExitCode;

use clap::Parser;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use time_expr::prelude::*;

use cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.is_repl() {
        return repl(&cli);
    }

    let source = match cli.source() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let input = source.text.trim();

    let expr = match parse_rich(input) {
        Ok(expr) => expr,
        Err(errs) => {
            report(input, &source.name, errs, &mut std::io::stderr())
                .expect("write diagnostics to stderr");
            return ExitCode::FAILURE;
        }
    };

    let value = match expr.eval() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    print_value(&value, &cli);
    ExitCode::SUCCESS
}

fn repl(cli: &Cli) -> ExitCode {
    let mut rl = match DefaultEditor::new() {
        Ok(rl) => rl,
        Err(e) => {
            eprintln!("Error: failed to initialize REPL: {e}");
            return ExitCode::FAILURE;
        }
    };

    loop {
        match rl.readline("> ") {
            Ok(line) => {
                let input = line.trim();

                if input.is_empty() {
                    continue;
                }

                if matches!(input, "quit" | "exit") {
                    break;
                }

                let _ = rl.add_history_entry(&line);

                let expr = match parse(input) {
                    Ok(expr) => expr,
                    Err(e) => {
                        eprintln!("Error: {e}");
                        continue;
                    }
                };

                match expr.eval() {
                    Ok(value) => print_value(&value, cli),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C: cancel current line, don't exit.
                continue;
            }
            Err(ReadlineError::Eof) => {
                // Ctrl-D: exit cleanly.
                break;
            }
            Err(e) => {
                eprintln!("Error: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    ExitCode::SUCCESS
}

fn print_value(value: &Value, cli: &Cli) {
    if cli.raw {
        print!("{value:?}");
    } else {
        print!("{value}");
    }

    if !cli.no_newline {
        println!();
    }
}
