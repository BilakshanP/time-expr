mod cli;

use std::process::ExitCode;

use clap::Parser;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use time_expr::prelude::*;

use cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Parse --var flags into the initial environment.
    let mut env = match cli.build_env() {
        Ok(env) => env,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if cli.is_repl() {
        return repl(&cli, &mut env);
    }

    let source = match cli.source() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let input = source.text.trim();

    let program = match parse_program(input) {
        Ok(prog) => prog,
        Err(e) => {
            eprintln!("Error[{}]: {e}", source.name);
            return ExitCode::FAILURE;
        }
    };

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

fn repl(cli: &Cli, env: &mut Env) -> ExitCode {
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

                let program = match parse_program(input) {
                    Ok(prog) => prog,
                    Err(e) => {
                        eprintln!("Error: {e}");
                        continue;
                    }
                };

                match program.eval(env) {
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
