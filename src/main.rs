//! A 20x20 Tapa puzzle game with a built-in solver and unique-solution generator.
//!
//! Usage:
//!   tapa                     launch the game (left click = wall, right click = empty)
//!   tapa --print [N]         generate and print N puzzles with their solutions
//!   tapa --bench [N]         time N generations
//!   tapa --config FILE       read generation settings from a file
//!   tapa --set KEY=VALUE     override one setting (repeatable)
//!   tapa --seed N            fixed seed
//!
//! Run `tapa --help` for the full list of settings.

mod cli;
mod config;
mod generator;
mod model;
mod render;
mod settings_ui;
mod rng;
mod solver;
mod window;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use config::Settings;

fn usage() {
    println!(
        "tapa {}\n\
         \n\
         usage:\n\
         \x20 tapa [options]             play in a window\n\
         \x20 tapa --print [N] [options] print N generated puzzles + solutions\n\
         \x20 tapa --bench [N] [options] time N generations\n\
         \n\
         options:\n\
         \x20 --config FILE            load settings from a 'key = value' file\n\
         \x20 --set KEY=VALUE          override one setting (repeatable)\n\
         \x20 --size N                 board is N x N (3..60)\n\
         \x20 --seed N                 fixed seed; omit for a clock-based seed\n\
         \x20 --density LO-HI          fraction of black cells, e.g. 0.40-0.50\n\
         \n\
         settings (all optional):\n\
         \x20 size, seed, density, max_clueless_fraction,\n\
         \x20 per_check_ms, per_check_nodes, total_nodes,\n\
         \x20 shape_check_ms, shape_check_nodes, max_attempts\n\
         \n\
         controls:\n\
         \x20 left click   mark wall (click the wall again to clear)\n\
         \x20 right click  mark empty (click it again to clear)\n\
         \x20 drag         paint a whole stroke\n\
         \x20 N new puzzle, R clear, C check, S solution, Z undo, Esc quit",
        env!("CARGO_PKG_VERSION")
    );
    println!("\nsettings file example:\n{}", config::EXAMPLE);
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mode = "gui";
    let mut count = 1usize;
    let mut config_path: Option<String> = None;
    let mut overrides: Vec<(String, String)> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--print" | "--bench" => {
                mode = if arg == "--print" { "print" } else { "bench" };
                if let Some(next) = args.get(i + 1) {
                    if let Ok(v) = next.parse::<usize>() {
                        count = v.max(1);
                        i += 1;
                    }
                }
            }
            "--config" => {
                let Some(next) = args.get(i + 1) else {
                    eprintln!("--config needs a file path");
                    return ExitCode::FAILURE;
                };
                config_path = Some(next.clone());
                i += 1;
            }
            "--set" => {
                let Some(next) = args.get(i + 1) else {
                    eprintln!("--set needs KEY=VALUE");
                    return ExitCode::FAILURE;
                };
                match next.split_once('=') {
                    Some((k, v)) => overrides.push((k.trim().to_string(), v.trim().to_string())),
                    None => {
                        eprintln!("--set expects KEY=VALUE, got '{next}'");
                        return ExitCode::FAILURE;
                    }
                }
                i += 1;
            }
            "--size" | "--seed" | "--density" => {
                let Some(next) = args.get(i + 1) else {
                    eprintln!("{arg} needs a value");
                    return ExitCode::FAILURE;
                };
                overrides.push((arg.trim_start_matches("--").to_string(), next.clone()));
                i += 1;
            }
            "-h" | "--help" => {
                usage();
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument: {other}\n");
                usage();
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let mut settings = Settings::load_default();
    if let Some(path) = &config_path {
        if let Err(err) = settings.load_file(Path::new(path)) {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    }
    for (key, value) in &overrides {
        if let Err(err) = settings.set(key, value) {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    }

    let settings_path = config_path
        .map(PathBuf::from)
        .unwrap_or_else(Settings::resolve_path);

    let seed = settings.seed.unwrap_or_else(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5EED)
    });

    match mode {
        "print" => {
            cli::print_puzzles(count, &settings, seed);
            ExitCode::SUCCESS
        }
        "bench" => {
            cli::bench(count, &settings, seed);
            ExitCode::SUCCESS
        }
        _ => match window::run(seed, &settings, settings_path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("failed to start the window: {err}");
                ExitCode::FAILURE
            }
        },
    }
}




