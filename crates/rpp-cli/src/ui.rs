//! Small console-output helpers built on `console` for consistent, pretty CLI
//! output (phase headers, timings, byte counts).

use std::time::Duration;

use console::style;

/// Print a top-level phase line, e.g. `▶ Resolving plugins`.
pub fn phase(label: &str) {
    println!("{} {}", style("▶").cyan().bold(), style(label).bold());
}

/// Print an indented detail line under a phase.
pub fn detail(text: impl AsRef<str>) {
    println!("  {}", style(text.as_ref()).dim());
}

/// Print a success line, e.g. `✓ Build complete in 1.2s`.
pub fn success(text: impl AsRef<str>) {
    println!("{} {}", style("✓").green().bold(), text.as_ref());
}

/// Print a warning line.
pub fn warn(text: impl AsRef<str>) {
    eprintln!("{} {}", style("!").yellow().bold(), text.as_ref());
}

/// Format a [`Duration`] compactly (`1.23s`, `850ms`).
pub fn fmt_duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs >= 1.0 {
        format!("{secs:.2}s")
    } else {
        format!("{}ms", d.as_millis())
    }
}

/// Format a byte count in human units (`1.2 KiB`, `3.4 MiB`).
pub fn fmt_bytes(bytes: u64) -> String {
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
pub fn fmt_savings(before: u64, after: u64) -> String {
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
