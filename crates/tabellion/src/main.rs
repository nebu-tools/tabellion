//! The tabellion command line.

mod inspect;
mod layout;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use nebu_docx::{Document, Revisions, TextOptions};

/// Exit codes. Usage errors exit with 2, from the argument parser.
const DIFFERS: u8 = 1;
const FAILED: u8 = 3;
const INTERNAL: u8 = 70;

#[derive(Parser)]
#[command(
    name = "tabellion",
    version,
    about = "Build and edit Word documents without Word",
    after_help = "Exit status: 0 on success, 1 if roundtrip finds the copy differs, 2 for a \
                  wrong command line, 3 if a file cannot be read or written or is not a Word \
                  document tabellion can read, 70 for an internal error."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show what a document holds: an outline, or one view of it.
    ///
    /// Without a view, prints the headings with their level, the tables and
    /// figures with their ids and captions, and counts of comments,
    /// revisions, fields and notes.
    Inspect {
        /// The document.
        file: PathBuf,
        /// The view to print.
        view: Option<View>,
        /// Print JSON in the shape crates/nebu-docx/schema/views.json gives.
        #[arg(long)]
        json: bool,
    },
    /// Print the text of a document as a reader sees it.
    ///
    /// Paragraphs are lines; a table's rows are lines of tab-separated
    /// cells. Field results are text, field codes are not.
    Text {
        /// The document.
        file: PathBuf,
        /// Which text tracked changes leave: accepted (inserted text in,
        /// deleted text out), rejected (the other way round) or current
        /// (both).
        #[arg(long, value_enum, default_value = "accepted")]
        revisions: RevisionView,
        /// Print each section's header before it and its footer after it.
        #[arg(long)]
        headers_footers: bool,
        /// Mark note references as [1] and print the notes at the end.
        #[arg(long)]
        notes: bool,
        /// Print the comments at the end.
        #[arg(long)]
        comments: bool,
        /// Leave out the text of text boxes.
        #[arg(long)]
        no_text_boxes: bool,
    },
    /// Open a document, save it unchanged, and compare the two files.
    ///
    /// Exits with 0 if OUT is IN byte for byte, and 1 if they differ.
    Roundtrip {
        /// The document to read.
        input: PathBuf,
        /// Where to write the copy.
        output: PathBuf,
    },
}

/// The views `inspect` prints.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum View {
    Blocks,
    Styles,
    Tables,
    Fields,
    Comments,
    Revisions,
    Images,
    Bookmarks,
    Notes,
    Sections,
    Numbering,
    ContentControls,
}

#[derive(Clone, Copy, ValueEnum)]
enum RevisionView {
    Accepted,
    Rejected,
    Current,
}

fn open(path: &Path) -> Result<Document, String> {
    Document::open(path).map_err(|e| match e {
        nebu_docx::Error::Io { source, .. } => format!("cannot read {}: {source}", path.display()),
        e => format!("{}: {e}", path.display()),
    })
}

fn text(file: &Path, options: &TextOptions) -> Result<(), String> {
    let mut doc = open(file)?;
    let t = doc
        .text(options)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    layout::emit(&t);
    Ok(())
}

fn roundtrip(input: &Path, output: &Path) -> Result<bool, String> {
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
    // The engine does not panic on any input; should a bug make it panic
    // all the same, say so in one line.
    std::panic::set_hook(Box::new(|info| {
        let place = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        eprintln!("tabellion: internal error{place}; please report it");
    }));
    let cli = Cli::parse();
    if let Command::Inspect {
        view: None,
        json: true,
        ..
    } = &cli.command
    {
        Cli::command()
            .error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "--json prints one view; name it, for example `inspect FILE blocks --json`",
            )
            .exit();
    }
    let run = || -> Result<bool, String> {
        match &cli.command {
            Command::Inspect { file, view, json } => inspect::run(file, *view, *json).map(|_| true),
            Command::Text {
                file,
                revisions,
                headers_footers,
                notes,
                comments,
                no_text_boxes,
            } => {
                let options = TextOptions {
                    revisions: match revisions {
                        RevisionView::Accepted => Revisions::Accepted,
                        RevisionView::Rejected => Revisions::Rejected,
                        RevisionView::Current => Revisions::Current,
                    },
                    headers_footers: *headers_footers,
                    notes: *notes,
                    comments: *comments,
                    text_boxes: !*no_text_boxes,
                };
                text(file, &options).map(|_| true)
            }
            Command::Roundtrip { input, output } => roundtrip(input, output),
        }
    };
    match std::panic::catch_unwind(run) {
        Ok(Ok(true)) => ExitCode::SUCCESS,
        Ok(Ok(false)) => ExitCode::from(DIFFERS),
        Ok(Err(e)) => {
            eprintln!("tabellion: {e}");
            ExitCode::from(FAILED)
        }
        Err(_) => ExitCode::from(INTERNAL),
    }
}
