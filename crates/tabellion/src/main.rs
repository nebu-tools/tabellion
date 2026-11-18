//! The tabellion command line.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nebu_docx::Document;

#[derive(Parser)]
#[command(
    name = "tabellion",
    version,
    about = "Build and edit Word documents without Word"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Open a document, save it unchanged, and compare the two files.
    ///
    /// Exits with 0 if OUT is IN byte for byte, 1 if they differ, and 2 if a
    /// file cannot be read or written.
    Roundtrip {
        /// The document to read.
        input: PathBuf,
        /// Where to write the copy.
        output: PathBuf,
    },
}

fn roundtrip(input: &PathBuf, output: &PathBuf) -> Result<bool, String> {
    let before =
        std::fs::read(input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    let doc =
        Document::from_bytes(before.clone()).map_err(|e| format!("{}: {e}", input.display()))?;
    doc.save(output).map_err(|e| e.to_string())?;
    let after =
        std::fs::read(output).map_err(|e| format!("cannot read {}: {e}", output.display()))?;
    if after == before {
        println!(
            "{}: identical to {}, all {} bytes",
            output.display(),
            input.display(),
            before.len()
        );
        return Ok(true);
    }
    let at = before
        .iter()
        .zip(&after)
        .take_while(|(a, b)| a == b)
        .count();
    println!(
        "{}: differs from {} from byte {at} on ({} bytes before, {} after)",
        output.display(),
        input.display(),
        before.len(),
        after.len()
    );
    Ok(false)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match &cli.command {
        Command::Roundtrip { input, output } => roundtrip(input, output),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("tabellion: {e}");
            ExitCode::from(2)
        }
    }
}
