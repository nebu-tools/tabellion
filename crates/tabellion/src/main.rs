//! The tabellion command line.

use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nebu_docx::Document;
use zip::ZipArchive;

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
    /// Open a document, save it unchanged, and compare the two files entry
    /// by entry.
    ///
    /// Exits with 0 if every zip entry of OUT is identical to the one in IN,
    /// 1 if some differ, and 2 if a file cannot be read or written.
    Roundtrip {
        /// The document to read.
        input: PathBuf,
        /// Where to write the copy.
        output: PathBuf,
    },
}

/// One zip entry as stored: name, method, checksum and compressed bytes.
#[derive(PartialEq, Eq)]
struct Stored {
    name: String,
    method: String,
    crc: u32,
    data: Vec<u8>,
}

fn stored_entries(bytes: &[u8]) -> Result<Vec<Stored>, String> {
    let mut a = ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    (0..a.len())
        .map(|i| {
            let mut f = a.by_index_raw(i).map_err(|e| e.to_string())?;
            let mut data = vec![];
            f.read_to_end(&mut data).map_err(|e| e.to_string())?;
            let name = f.name().map_err(|e| e.to_string())?.into_owned();
            Ok(Stored {
                name,
                method: format!("{:?}", f.compression()),
                crc: f.crc32(),
                data,
            })
        })
        .collect()
}

fn roundtrip(input: &PathBuf, output: &PathBuf) -> Result<bool, String> {
    let before =
        std::fs::read(input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    let doc =
        Document::from_bytes(before.clone()).map_err(|e| format!("{}: {e}", input.display()))?;
    doc.save(output).map_err(|e| e.to_string())?;
    let after =
        std::fs::read(output).map_err(|e| format!("cannot read {}: {e}", output.display()))?;
    let a = stored_entries(&before)?;
    let b = stored_entries(&after)?;
    let mut differ = vec![];
    for e in &a {
        match b.iter().find(|x| x.name == e.name) {
            Some(x) if x == e => {}
            Some(_) => differ.push(format!("{}: stored differently", e.name)),
            None => differ.push(format!("{}: missing", e.name)),
        }
    }
    for x in b.iter().filter(|x| !a.iter().any(|e| e.name == x.name)) {
        differ.push(format!("{}: added", x.name));
    }
    if differ.is_empty() {
        println!("{}: all {} entries identical", output.display(), a.len());
        return Ok(true);
    }
    println!(
        "{}: {} of {} entries differ",
        output.display(),
        differ.len(),
        a.len()
    );
    for d in differ {
        println!("  {d}");
    }
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
