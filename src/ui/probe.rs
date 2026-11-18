//! Background `ssh` runs for Monitor and Agents.
//!
//! What comes back is written by the far side, so it is bounded both ways: a
//! host that never answers cannot leave a card waiting forever, and one that
//! answers without end cannot fill memory.

use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};

const TIMEOUT: Duration = Duration::from_secs(10);
/// Far more than any probe prints; `top` included it stays under 10 KiB.
const OUTPUT_LIMIT: usize = 256 * 1024;
/// Only the first line of ssh's complaint is shown.
const ERROR_LIMIT: usize = 8 * 1024;

pub async fn run(command: &[String], environment: &[(String, String)]) -> Result<String, String> {
    run_within(TIMEOUT, command, environment).await
}

async fn run_within(
    timeout: Duration,
    command: &[String],
    environment: &[(String, String)],
) -> Result<String, String> {
    let args: Vec<&std::ffi::OsStr> =
        command.iter().map(std::ffi::OsStr::new).collect();

    let launcher = gio::SubprocessLauncher::new(
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    );

    // With no channel open yet, the keyring may answer - but only for this
    // host, which is what the environment tells askpass.
    for (key, value) in environment {
        launcher.setenv(std::ffi::OsStr::new(key), std::ffi::OsStr::new(value), true);
    }

    let process = launcher
        .spawn(&args)
        .map_err(|error| error.message().to_string())?;

    let (Some(stdout), Some(stderr)) = (process.stdout_pipe(), process.stderr_pipe()) else {
        process.force_exit();
        return Err("could not read from ssh".into());
    };

    // Both pipes are drained at once; reading one to the end first would let
    // the other fill up and stall ssh.
    let errors = glib::spawn_future_local(read_kept(stderr, ERROR_LIMIT));
    let finished = {
        let process = process.clone();
        async move {
            let output = read_limited(stdout, OUTPUT_LIMIT).await?;
            process.wait_future().await.map_err(|error| error.message().to_string())?;
            Ok::<_, String>(output)
        }
    };

    let output = match glib::future_with_timeout(timeout, finished).await {
        Ok(Ok(output)) => output,
        Ok(Err(reason)) => {
            process.force_exit();
            return Err(reason);
        }
        Err(_) => {
            process.force_exit();
            return Err(format!("no answer within {} s", timeout.as_secs()));
        }
    };

    if process.is_successful() {
        return Ok(String::from_utf8_lossy(&output).into_owned());
    }

    // ssh's own message is what helps the user; the first line is enough.
    let message = errors
        .await
        .ok()
        .and_then(Result::ok)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let message = message.lines().next().unwrap_or_default().trim();

    Err(if message.is_empty() { "connection failed".into() } else { message.to_string() })
}

/// Reads a pipe to the end, keeping only the first `keep` bytes. The rest is
/// read and dropped: stopping would leave ssh blocked on a full pipe.
async fn read_kept(stream: gio::InputStream, keep: usize) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();

    loop {
        let chunk = stream
            .read_bytes_future(16 * 1024, glib::Priority::DEFAULT)
            .await
            .map_err(|error| error.message().to_string())?;

        if chunk.is_empty() {
            return Ok(data);
        }

        let room = keep.saturating_sub(data.len());
        data.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
}

/// Reads a pipe to the end, giving up once it passes `limit` bytes.
async fn read_limited(stream: gio::InputStream, limit: usize) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();

    loop {
        let chunk = stream
            .read_bytes_future(16 * 1024, glib::Priority::DEFAULT)
            .await
            .map_err(|error| error.message().to_string())?;

        if chunk.is_empty() {
            return Ok(data);
        }

        data.extend_from_slice(&chunk);
        if data.len() > limit {
            return Err("the host sent far more output than expected".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str, timeout: Duration) -> Result<String, String> {
        let command = ["sh".to_string(), "-c".to_string(), script.to_string()];
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| context.block_on(run_within(timeout, &command, &[])))
            .unwrap()
    }

    #[test]
    fn output_and_errors_come_back() {
        assert_eq!(sh("echo hi", TIMEOUT), Ok("hi\n".into()));
        assert_eq!(sh("echo first >&2; echo second >&2; exit 3", TIMEOUT), Err("first".into()));
    }

    #[test]
    fn a_silent_host_is_given_up_on() {
        let started = std::time::Instant::now();
        let result = sh("sleep 30", Duration::from_secs(1));
        assert_eq!(result, Err("no answer within 1 s".into()));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn endless_output_is_cut_off() {
        let result = sh("yes runetty", TIMEOUT);
        assert!(result.is_err_and(|reason| reason.contains("more output")));
    }

    /// A full stderr pipe must not stall a probe whose stdout is small.
    #[test]
    fn a_noisy_stderr_does_not_stall() {
        assert_eq!(sh("head -c 200000 /dev/zero >&2; echo ok", TIMEOUT), Ok("ok\n".into()));
    }
}
