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
                  wrong command line, 3 if a file cannot be read or written, is not a Word \
                  document tabellion can read, or holds content that cannot be written where \
                  asked, 70 for an internal error."
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
    /// Insert content from the content model's JSON form.
    ///
    /// The place is one of --before, --after or --replace with a locator
    /// (p:ID, t:ID, #INDEX, heading:TEXT, bookmark:NAME, placeholder:NAME
    /// or text:TEXT), or --at NAME for the paragraph that holds {{NAME}};
    /// without one, the content goes at the end of the body. Pictures given
    /// as files are read relative to the JSON file.
    #[command(group(clap::ArgGroup::new("place").args(["before", "after", "replace", "at"])))]
    #[command(group(clap::ArgGroup::new("source").required(true).args(["content", "typst"])))]
    Insert {
        /// The document to read.
        input: PathBuf,
        /// Where to write the result.
        output: PathBuf,
        /// The content: JSON in the shape crates/nebu-docx/schema/content.json gives.
        #[arg(long, value_name = "FILE")]
        content: Option<PathBuf>,
        /// The content: Typst source with headings, paragraphs and lists.
        #[arg(long, value_name = "FILE")]
        typst: Option<PathBuf>,
        /// Before the block the locator names.
        #[arg(long, value_name = "LOCATOR")]
        before: Option<String>,
        /// After the block the locator names.
        #[arg(long, value_name = "LOCATOR")]
        after: Option<String>,
        /// Instead of the block the locator names.
        #[arg(long, value_name = "LOCATOR")]
        replace: Option<String>,
        /// Instead of the paragraph that holds {{NAME}}.
        #[arg(long, value_name = "NAME")]
        at: Option<String>,
        /// Write the new blocks as a tracked insertion by this author.
        #[arg(long, value_name = "AUTHOR")]
        author: Option<String>,
    },
    /// Update the results of fields without Word.
    ///
    /// Recomputes every SEQ and bare REF result in the body; fields only
    /// Word can compute keep their results. Prints how many fields changed.
    Fields {
        /// The document to read.
        input: PathBuf,
        /// Where to write the result.
        output: PathBuf,
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
    Paragraphs,
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
        .views()
        .map_err(|e| format!("{}: {e}", file.display()))?
        .text(options);
    layout::emit(&t);
    Ok(())
}

struct InsertArgs<'a> {
    input: &'a Path,
    output: &'a Path,
    content: Option<&'a Path>,
    typst: Option<&'a Path>,
    before: &'a Option<String>,
    after: &'a Option<String>,
    replace: &'a Option<String>,
    at: &'a Option<String>,
    author: &'a Option<String>,
}

fn insert(a: InsertArgs<'_>) -> Result<bool, String> {
    use nebu_docx::{Content, Locator, Place, WriteOptions};
    let mut doc = open(a.input)?;
    // Exactly one source: the argument group enforces it.
    let (source, content) = match (a.content, a.typst) {
        (Some(path), None) => {
            let json =
                std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let content =
                Content::from_json(&json).map_err(|e| format!("{}: {e}", path.display()))?;
            (path, content)
        }
        (None, Some(path)) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let content =
                tabellion_typst::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            (path, content)
        }
        _ => return Err("give exactly one of --content and --typst".to_string()),
    };
    let at = |e: nebu_docx::Error| format!("{}: {e}", a.input.display());
    let place = {
        let mut found = |loc: &str| -> Result<nebu_docx::xml::NodeId, String> {
            let l: Locator = loc.parse().map_err(|e: nebu_docx::Error| e.to_string())?;
            Ok(doc.views().map_err(at)?.locate(&l).map_err(at)?.node)
        };
        match (a.before, a.after, a.replace, a.at) {
            (Some(l), ..) => Place::Before(found(l)?),
            (_, Some(l), ..) => Place::After(found(l)?),
            (_, _, Some(l), _) => Place::Replace(found(l)?),
            (.., Some(name)) => {
                let l = Locator::placeholder(name).map_err(|e| e.to_string())?;
                Place::Replace(doc.views().map_err(at)?.locate(&l).map_err(at)?.node)
            }
            _ => Place::End,
        }
    };
    let dir = source
        .parent()
        .map(|p| {
            if p.as_os_str().is_empty() {
                Path::new(".")
            } else {
                p
            }
        })
        .map(Path::to_path_buf);
    let mut opts = WriteOptions::for_content(&content, dir);
    if let Some(author) = a.author {
        use nebu_docx::Track;
        opts.track = Some(Track::now(author));
    }
    let done = doc
        .insert(place, &content.blocks, &opts)
        .map_err(|e| format!("{}: {e}", source.display()))?;
    doc.save(a.output).map_err(|e| e.to_string())?;
    let n = content.blocks.len();
    let mut line = format!(
        "{}: inserted {n} {} from {}",
        a.output.display(),
        if n == 1 { "block" } else { "blocks" },
        source.display()
    );
    if !done.styles_added.is_empty() {
        line.push_str(&format!(
            "; added the styles {}",
            done.styles_added.join(", ")
        ));
    }
    if !done.labels.is_empty() {
        let names: Vec<&str> = done.labels.iter().map(|(l, _)| l.as_str()).collect();
        line.push_str(&format!("; labels {}", names.join(", ")));
    }
    if done.renumbered > 0 {
        line.push_str(&format!("; renumbered {} later fields", done.renumbered));
    }
    println!("{line}");
    Ok(true)
}

fn fields(input: &Path, output: &Path) -> Result<bool, String> {
    let mut doc = open(input)?;
    let n = doc
        .update_fields()
        .map_err(|e| format!("{}: {e}", input.display()))?;
    doc.save(output).map_err(|e| e.to_string())?;
    println!(
        "{}: updated {n} {}",
        output.display(),
        if n == 1 { "field" } else { "fields" }
    );
    Ok(true)
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
            Command::Fields { input, output } => fields(input, output),
            Command::Insert {
                input,
                output,
                content,
                typst,
                before,
                after,
                replace,
                at,
                author,
            } => insert(InsertArgs {
                input,
                output,
                content: content.as_deref(),
                typst: typst.as_deref(),
                before,
                after,
                replace,
                at,
                author,
            }),
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
