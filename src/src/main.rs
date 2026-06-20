#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::fmt;
use std::io::Write;

fn main() {
    match j3pic::run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            report_startup_error(&error);
            std::process::exit(1);
        }
    }
}

fn report_startup_error(error: &impl fmt::Display) {
    let mut stderr = std::io::stderr();
    let _ = writeln!(stderr, "j3Pic failed: {error}");

    #[cfg(target_os = "windows")]
    {
        let message = format!("Could not start j3Pic.\n\n{error}");
        j3pic::platform::win32::show_startup_error_message(&message);
    }
}
