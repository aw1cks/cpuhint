use std::{env, fs, process::ExitCode};

use cpuhint_core::parse_cpu_list;
use cpuhint_linux::resolve_online_cpu_list;

const DEFAULT_ONLINE_PATH: &str = "/sys/devices/system/cpu/online";

fn main() -> ExitCode {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_ONLINE_PATH.to_owned());

    let input = match fs::read(&path) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("failed to read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };

    println!("online CPU source: {path}");
    print!("raw CPU list: {}", String::from_utf8_lossy(&input));
    if !input.ends_with(b"\n") {
        println!();
    }

    match parse_cpu_list(&input) {
        Ok(list) => {
            println!("parsed CPU count: {}", list.count());
            println!("contiguous virtual IDs: {}", list.is_contiguous_from_zero());
            if path == DEFAULT_ONLINE_PATH {
                match resolve_online_cpu_list() {
                    Ok(resolved) => println!("raw-syscall resolver count: {}", resolved.count()),
                    Err(error) => {
                        eprintln!("raw-syscall resolver failed: {error:?}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("invalid CPU list: {error}");
            ExitCode::FAILURE
        }
    }
}
