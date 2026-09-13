//! Cliclack prompts and status output on stderr, with plain output for scripts.

use std::io::{self, IsTerminal};
use std::time::Duration;

/// Whether stderr supports terminal output.
pub fn is_terminal() -> bool {
    io::stderr().is_terminal() && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
}

pub(crate) fn is_interactive() -> bool {
    io::stdin().is_terminal() && is_terminal()
}

pub(crate) fn input(label: &str, default: &str) -> io::Result<String> {
    cliclack::input(label).default_input(default).interact()
}

pub(crate) fn select<T: Clone + Eq>(label: &str, items: &[(T, &str, &str)]) -> io::Result<T> {
    cliclack::select(label).items(items).interact()
}

fn display(text: &str, prefix: &str, render: impl FnOnce(&str) -> io::Result<()>) {
    if is_terminal() {
        let _ = render(text);
    } else {
        eprintln!("{prefix}{text}");
    }
}

/// Start a command's status output.
pub(crate) fn intro(text: impl AsRef<str>) {
    display(text.as_ref(), "", |text| cliclack::intro(text));
}

/// Print a top-level phase line.
pub(crate) fn phase(label: &str) {
    display(label, "", |text| cliclack::log::step(text));
}

/// Print a detail line under a phase.
pub(crate) fn detail(text: impl AsRef<str>) {
    display(text.as_ref(), "  ", |text| cliclack::log::remark(text));
}

/// Finish a command's status output successfully.
pub(crate) fn success(text: impl AsRef<str>) {
    display(text.as_ref(), "", |text| cliclack::outro(text));
}

/// Print a warning line.
pub(crate) fn warn(text: impl AsRef<str>) {
    display(text.as_ref(), "warning: ", |text| {
        cliclack::log::warning(text)
    });
}

/// Finish with an error, including its context chain.
pub fn error(text: impl AsRef<str>) {
    display(text.as_ref(), "error: ", |text| {
        cliclack::outro_cancel(text)
    });
}

/// Finish a cancelled prompt.
pub fn cancel() {
    display("Cancelled", "", |text| cliclack::outro_cancel(text));
}

/// Format a [`Duration`] compactly (`1.23s`, `850ms`).
pub(crate) fn fmt_duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs >= 1.0 {
        format!("{secs:.2}s")
    } else {
        format!("{}ms", d.as_millis())
    }
}

/// Format a byte count in human units (`1.2 KiB`, `3.4 MiB`).
pub(crate) fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Format a savings percentage given before/after byte counts.
pub(crate) fn fmt_savings(before: u64, after: u64) -> String {
    if before == 0 {
        return "0%".to_string();
    }
    let saved = before.saturating_sub(after) as f64 / before as f64 * 100.0;
    format!("{saved:.1}%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_formatting() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(2048), "2.0 KiB");
    }

    #[test]
    fn savings_formatting() {
        assert_eq!(fmt_savings(1000, 750), "25.0%");
        assert_eq!(fmt_savings(0, 0), "0%");
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(fmt_duration(Duration::from_millis(850)), "850ms");
        assert_eq!(fmt_duration(Duration::from_millis(1500)), "1.50s");
    }
}
