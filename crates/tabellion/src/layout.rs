//! Aligned text output that fits the terminal.

use std::io::Write;

/// Writes to standard output. A reader that stops reading, such as
/// `head`, ends the program quietly.
pub(crate) fn emit(s: &str) {
    let mut out = std::io::stdout().lock();
    if let Err(e) = out.write_all(s.as_bytes()).and_then(|_| out.flush()) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        eprintln!("tabellion: cannot write the output: {e}");
        std::process::exit(3);
    }
}

/// The terminal width from `COLUMNS`, else 100.
pub(crate) fn width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse::<usize>().ok())
        .filter(|&c| c >= 40)
        .unwrap_or(100)
}

/// Text on one line: line ends and tabs become spaces, and control
/// characters are left out.
pub(crate) fn one_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' | '\r' | '\t' | '\u{0B}' | '\u{0C}' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// Cuts text to `n` characters, ending in `…` when cut.
pub(crate) fn cut(s: &str, n: usize) -> String {
    if chars(s) <= n {
        return s.to_string();
    }
    if n == 0 {
        return String::new();
    }
    let mut t: String = s.chars().take(n - 1).collect();
    t.push('…');
    t
}

/// Rows of cells in columns. Every column is as wide as its widest cell,
/// except the last, which takes the width that is left and is cut to it.
pub(crate) fn columns(header: &[&str], rows: &[Vec<String>]) -> String {
    let n = header.len();
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| r.iter().map(|c| one_line(c)).collect())
        .collect();
    let mut w: Vec<usize> = header.iter().map(|h| chars(h)).collect();
    for r in &rows {
        for (i, c) in r.iter().enumerate().take(n) {
            w[i] = w[i].max(chars(c));
        }
    }
    // Columns before the last are capped, so the last keeps some room.
    let total = width();
    for x in w.iter_mut().take(n.saturating_sub(1)) {
        *x = (*x).min(40);
    }
    let used: usize = w.iter().take(n.saturating_sub(1)).map(|x| x + 2).sum();
    if n > 0 {
        w[n - 1] = w[n - 1].min(total.saturating_sub(used).max(10));
    }
    let mut out = String::new();
    let mut line = |cells: Vec<String>| {
        let mut l = String::new();
        for (i, c) in cells.iter().enumerate().take(n) {
            let c = cut(c, w[i]);
            if i + 1 < n {
                l.push_str(&c);
                l.push_str(&" ".repeat(w[i] - chars(&c) + 2));
            } else {
                l.push_str(&c);
            }
        }
        out.push_str(l.trim_end());
        out.push('\n');
    };
    line(header.iter().map(|h| h.to_string()).collect());
    for r in rows {
        line(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_align_and_cut() {
        let rows = vec![
            vec!["1".to_string(), "short".to_string()],
            vec!["22".to_string(), "x".repeat(300)],
        ];
        let s = columns(&["N", "TEXT"], &rows);
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines[0], "N   TEXT");
        assert_eq!(lines[1], "1   short");
        assert!(lines[2].ends_with('…'));
        assert!(lines.iter().all(|l| l.chars().count() <= width()));
    }

    #[test]
    fn one_line_drops_controls() {
        assert_eq!(one_line("a\tb\nc\u{7}d"), "a b cd");
    }
}
