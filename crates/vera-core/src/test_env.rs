//! Helpers for running environment-dependent tests in child processes.

/// Run an ignored test probe with an isolated environment.
///
/// Environment changes made through [`std::process::Command`] only affect the
/// child, so the parent test process and its other test threads never observe
/// a mutation. The probe's output is included when it fails so the assertion
/// that failed remains visible to the parent test.
pub(crate) fn run_env_test(test_name: &str, vars: &[(&str, Option<&str>)]) {
    let mut command = std::process::Command::new(
        std::env::current_exe().expect("the test binary path must be available"),
    );
    command.args([test_name, "--exact", "--ignored", "--nocapture"]);
    for &(key, value) in vars {
        match value {
            Some(value) => {
                command.env(key, value);
            }
            None => {
                command.env_remove(key);
            }
        }
    }

    let output = command
        .output()
        .unwrap_or_else(|err| panic!("failed to run child test {test_name}: {err}"));
    assert!(
        output.status.success(),
        "child test {test_name} failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
