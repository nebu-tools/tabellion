//! Every command the README shows runs and prints what the README says.
//!
//! The commands are the `$ tabellion …` lines of its `console` blocks; the
//! lines after each, up to the next command or the end of the block, are
//! what it prints. A file the command writes (a `.docx` that is not in the
//! repository) goes to a directory of the test, and the output is read
//! with the file's name as the README gives it. Blank lines at the end of
//! an output part it from the next command and are not compared.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The commands of the README with what each prints.
fn examples() -> Vec<(String, String)> {
    let readme = std::fs::read_to_string(root().join("README.md")).unwrap();
    let mut out = vec![];
    let mut in_block = false;
    let mut current: Option<(String, String)> = None;
    for line in readme.lines() {
        if line.starts_with("```console") {
            in_block = true;
            continue;
        }
        if in_block && line.starts_with("```") {
            in_block = false;
            out.extend(current.take());
            continue;
        }
        if !in_block {
            continue;
        }
        if let Some(cmd) = line.strip_prefix("$ ") {
            out.extend(current.take());
            current = Some((cmd.to_string(), String::new()));
        } else if let Some((_, o)) = &mut current {
            o.push_str(line);
            o.push('\n');
        }
    }
    out
}

#[test]
fn the_readme_shows_what_the_commands_print() {
    let dir = tempfile::tempdir().unwrap();
    let examples = examples();
    assert!(examples.len() >= 6, "{} examples", examples.len());
    let mut failures = vec![];
    for (cmd, want) in examples {
        let words: Vec<&str> = cmd.split_whitespace().collect();
        assert_eq!(words[0], "tabellion", "{cmd}");
        let mut written = vec![];
        let args: Vec<String> = words[1..]
            .iter()
            .map(|w| {
                if w.ends_with(".docx") && !root().join(w).exists() {
                    let p = dir.path().join(w);
                    written.push((p.display().to_string(), w.to_string()));
                    p.display().to_string()
                } else {
                    w.to_string()
                }
            })
            .collect();
        let o = Command::new(env!("CARGO_BIN_EXE_tabellion"))
            .args(&args)
            .current_dir(root())
            .env("COLUMNS", "100")
            .output()
            .unwrap();
        let mut got = String::from_utf8(o.stdout).unwrap();
        for (path, name) in &written {
            got = got.replace(path.as_str(), name);
        }
        if !o.status.success() {
            failures.push(format!("{cmd}: {}", String::from_utf8_lossy(&o.stderr)));
        } else if got.trim_end() != want.trim_end() {
            // A blank line in the README may part two commands.
            failures.push(format!("{cmd}:\nREADME:\n{want}\nprinted:\n{got}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
