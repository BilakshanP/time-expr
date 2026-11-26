mod cli;

use std::process::ExitCode;

use clap::Parser;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use time_expr::parser;
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

    // Parse as a program (supports variables and ;).
    let program = match parser::parse_program_rich(input) {
        Ok(prog) => prog,
        Err(errs) => {
            report(input, &source.name, errs, &mut std::io::stderr())
                .expect("write diagnostics to stderr");
            return ExitCode::FAILURE;
        }
    };

    let mut env = Env::new();
    match program.eval(&mut env) {
        Ok(Some(value)) => print_value(&value, &cli),
        Ok(None) => {}
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    }

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

    // Variables persist across REPL lines.
    let mut env = Env::new();

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

                let program = match parse_program(input) {
                    Ok(prog) => prog,
                    Err(e) => {
                        eprintln!("Error: {e}");
                        continue;
                    }
                };

                match program.eval(&mut env) {
                    Ok(Some(value)) => print_value(&value, cli),
                    Ok(None) => {}
                    Err(e) => eprintln!("Error: {e}"),
                }
            }
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
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
