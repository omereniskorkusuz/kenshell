//! Ping / traceroute from this computer (P5-T05): validates the target, runs the OS
//! tool (argument lists in `platform::diag`, never through a shell) and streams its
//! output line by line until it ends or is stopped.

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;

use crate::error::{AppError, ErrorCode, Result};
use crate::platform::diag::Probe;

const MAX_TARGET: usize = 253;
/// Lines longer than this are cut (a tool never prints such lines normally).
const MAX_LINE: usize = 512;

/// A host name or IP address; nothing that could read as an option or carry spaces.
pub fn validate_target(target: &str) -> Result<&str> {
    let t = target.trim();
    let ok = !t.is_empty()
        && t.len() <= MAX_TARGET
        && !t.starts_with('-')
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_' | '%'));
    if ok {
        Ok(t)
    } else {
        Err(AppError::invalid("enter a host name or an IP address"))
    }
}

/// Runs the first probe whose program exists, sending each output line to `on_line`,
/// until it exits or `stop` turns true. Returns the exit code (None when stopped).
pub async fn run(
    probes: &[Probe],
    mut stop: watch::Receiver<bool>,
    on_line: &(dyn Fn(String) + Send + Sync),
) -> Result<Option<i32>> {
    let mut last_err = None;
    for p in probes {
        let mut cmd = tokio::process::Command::new(p.program);
        cmd.args(&p.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        // CREATE_NO_WINDOW: no console flashes up from a GUI app.
        cmd.creation_flags(0x0800_0000);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                last_err = Some(p.program);
                continue;
            }
            Err(e) => return Err(AppError::from(e)),
        };
        let mut out = child.stdout.take().map(|s| BufReader::new(s).lines());
        let mut err = child.stderr.take().map(|s| BufReader::new(s).lines());
        let emit = |line: String| {
            let mut line = line.trim_end().to_owned();
            if line.len() > MAX_LINE {
                let cut = (0..=MAX_LINE)
                    .rev()
                    .find(|i| line.is_char_boundary(*i))
                    .unwrap_or(0);
                line.truncate(cut);
            }
            on_line(line);
        };
        loop {
            tokio::select! {
                l = async { out.as_mut()?.next_line().await.ok().flatten() }, if out.is_some() => {
                    match l { Some(l) => emit(l), None => out = None }
                }
                l = async { err.as_mut()?.next_line().await.ok().flatten() }, if err.is_some() => {
                    match l { Some(l) => emit(l), None => err = None }
                }
                _ = stop.changed() => {
                    if *stop.borrow() {
                        let _ = child.kill().await;
                        return Ok(None);
                    }
                }
                status = child.wait(), if out.is_none() && err.is_none() => {
                    return Ok(status.map_err(AppError::from)?.code());
                }
            }
        }
    }
    Err(AppError::with(
        ErrorCode::Io,
        format!("{} is not installed", last_err.unwrap_or("the tool")),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets() {
        for ok in [
            "10.0.0.1",
            "core-sw1.lan",
            "fe80::1%en0",
            "2001:db8::1",
            "host_1",
        ] {
            assert!(validate_target(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "-c 1000 x",
            "a b",
            "x;rm",
            "$(id)",
            "a\nb",
            &"a".repeat(300),
        ] {
            assert!(validate_target(bad).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn streams_lines_falls_back_and_stops() {
        use std::sync::{Arc, Mutex};
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        let on_line = move |l: String| sink.lock().unwrap().push(l);
        let (_tx, rx) = watch::channel(false);
        let probes = [
            Probe {
                program: "kenshell-no-such-tool",
                args: vec![],
            },
            Probe {
                program: "sh",
                args: vec!["-c".into(), "echo one; echo two >&2; exit 3".into()],
            },
        ];
        assert_eq!(run(&probes, rx, &on_line).await.unwrap(), Some(3));
        let mut got = lines.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, ["one", "two"]);

        let (tx, rx) = watch::channel(false);
        let probes = [Probe {
            program: "sh",
            args: vec!["-c".into(), "echo start; sleep 30".into()],
        }];
        let started = std::time::Instant::now();
        let stopper = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = tx.send(true);
        });
        assert_eq!(run(&probes, rx, &|_| {}).await.unwrap(), None);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        stopper.await.unwrap();

        let (_tx, rx) = watch::channel(false);
        let missing = [Probe {
            program: "kenshell-no-such-tool",
            args: vec![],
        }];
        assert!(run(&missing, rx, &|_| {}).await.is_err());
    }
}
