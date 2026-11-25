use std::process::ExitCode;

// Release builds start without the Visual C++ runtime; see build.rs.
#[cfg(all(nativeboat_own_runtime, not(test)))]
extern crate nativeboat_std_start;

fn main() -> ExitCode {
    let code = nativeboat_cli::run(
        std::env::args_os(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    );
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entrypoint_returns_the_cli_status_without_terminating_the_test_process() {
        // The test harness arguments are invalid nativeboat arguments, so the
        // real entrypoint exercises the CLI error path and returns its status.
        assert_eq!(main(), ExitCode::from(2));
    }
}
