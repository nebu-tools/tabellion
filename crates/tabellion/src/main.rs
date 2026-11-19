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
    /// Write each table to a file of its own.
    ///
    /// One file per table in document order, named table-1.csv,
    /// table-2.csv and so on; nested tables get files of their own. A
    /// merged cell leaves its grid positions empty. Refuses to
    /// overwrite existing files.
    Tables {
        /// The document to read.
        file: PathBuf,
        /// The directory to write into, made when missing.
        dir: PathBuf,
        /// The file shape.
        #[arg(long, value_enum, default_value = "csv")]
        format: TableFormat,
    },
    /// Write each embedded picture to a file of its own.
    ///
    /// One file per image in document order, named after the part that
    /// holds it. Linked pictures have no bytes to write and are
    /// skipped. Refuses to overwrite existing files.
    Images {
        /// The document to read.
        file: PathBuf,
        /// The directory to write into, made when missing.
        dir: PathBuf,
    },
    /// Replace a table from a CSV file.
    ///
    /// The target names the table's label: the table under the caption
    /// that carries the label is replaced by the CSV rows, one row of
    /// text cells per record. The caption and the table style stay; a
    /// repeating header row does not carry over.
    Table {
        /// The document to read.
        input: PathBuf,
        /// Where to write the result.
        output: PathBuf,
        /// The label of the table to replace.
        #[arg(long, value_name = "LABEL")]
        target: String,
        /// The CSV file to read the table from.
        #[arg(long, value_name = "FILE")]
        csv: PathBuf,
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

/// The file shapes `tables` writes.
#[derive(Clone, Copy, ValueEnum)]
enum TableFormat {
    Csv,
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

fn tables(file: &Path, dir: &Path, format: TableFormat) -> Result<bool, String> {
    let mut doc = open(file)?;
    let views = doc
        .views()
        .map_err(|e| format!("{}: {e}", file.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot write {}: {e}", dir.display()))?;
    let mut count = 0;
    for table in views.tables() {
        count += 1;
        let name = match format {
            TableFormat::Csv => format!("table-{count}.csv"),
        };
        let path = dir.join(name);
        let file = std::fs::File::create_new(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                format!("{}: file exists, refusing to overwrite", path.display())
            } else {
                format!("cannot write {}: {e}", path.display())
            }
        })?;
        // The writer's default LF terminators; tests parse the files back.
        let mut out = csv::Writer::from_writer(file);
        for row in &table.rows {
            let record: Vec<&str> = row
                .iter()
                .map(|cell| cell.as_deref().unwrap_or(""))
                .collect();
            out.write_record(&record)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
        out.flush()
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    println!(
        "{}: wrote {count} {}",
        dir.display(),
        if count == 1 { "table" } else { "tables" }
    );
    Ok(true)
}

fn images(file: &Path, dir: &Path) -> Result<bool, String> {
    let mut doc = open(file)?;
    let parts: Vec<Option<String>> = doc
        .views()
        .map_err(|e| format!("{}: {e}", file.display()))?
        .images()
        .iter()
        .map(|image| image.part.clone())
        .collect();
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot write {}: {e}", dir.display()))?;
    let mut count = 0;
    let mut skipped = 0;
    let mut used = std::collections::HashSet::new();
    for part in &parts {
        let Some(part) = part else {
            skipped += 1;
            continue;
        };
        let base =
            safe_name(part).ok_or_else(|| format!("{part}: refusing to write unsafe file name"))?;
        let mut name = base.clone();
        let mut n = 2;
        while !used.insert(name.clone()) {
            name = numbered_name(&base, n);
            n += 1;
        }
        let path = dir.join(name);
        let bytes = doc
            .read_part(part)
            .map_err(|e| format!("{}: {e}", file.display()))?;
        std::fs::File::create_new(&path)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(&bytes)
            })
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    format!("{}: file exists, refusing to overwrite", path.display())
                } else {
                    format!("cannot write {}: {e}", path.display())
                }
            })?;
        count += 1;
    }
    let mut line = format!(
        "{}: wrote {count} {}",
        dir.display(),
        if count == 1 { "image" } else { "images" }
    );
    if skipped > 0 {
        line += &format!(
            ", skipped {skipped} linked {}",
            if skipped == 1 { "image" } else { "images" }
        );
    }
    println!("{line}");
    Ok(true)
}

/// True for a `SEQ` field of the table sequence, whatever the case of
/// its identifier: the writer numbers `SEQ Table` and `SEQ table` as
/// one sequence.
fn is_table_seq(field: &nebu_docx::Field) -> bool {
    field.kind == "SEQ"
        && field
            .code
            .split_whitespace()
            .nth(1)
            .is_some_and(|id| id.eq_ignore_ascii_case("Table"))
}

fn replace_table(input: &Path, output: &Path, target: &str, csv: &Path) -> Result<bool, String> {
    use nebu_docx::{BlockKind, Content, Locator, Place, WriteOptions, content};
    let mut doc = open(input)?;
    let at = |e: nebu_docx::Error| format!("{}: {e}", input.display());
    let blocks = doc.views().map_err(at)?.blocks().to_vec();
    // The label lives on the caption paragraph; the table follows it. The
    // bookmark alone is not enough: figures, headings and equations take
    // `_Ref_` bookmarks too. Only a `SEQ Table` caption names a table.
    let mark = format!("bookmark:_Ref_{target}");
    let label: Locator = mark.parse().map_err(|e: nebu_docx::Error| e.to_string())?;
    let caption = doc.views().map_err(at)?.locate(&label).map_err(at)?;
    let (caption_id, caption_index) = (caption.id, caption.index);
    let is_table_caption = doc
        .views()
        .map_err(at)?
        .fields()
        .iter()
        .any(|field| field.location.block.as_ref() == Some(&caption_id) && is_table_seq(field));
    let table = is_table_caption
        .then(|| blocks.get(caption_index + 1))
        .flatten()
        .filter(|block| block.kind == BlockKind::Table)
        .ok_or_else(|| format!("{}: no table labelled {target}", input.display()))?;
    let mut records = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_path(csv)
        .map_err(|e| format!("cannot read {}: {e}", csv.display()))?;
    let mut rows = Vec::new();
    for record in records.records() {
        let record = record.map_err(|e| format!("cannot read {}: {e}", csv.display()))?;
        rows.push(record.iter().map(str::to_string).collect::<Vec<_>>());
    }
    if rows.is_empty() {
        return Err(format!("cannot read {}: no rows", csv.display()));
    }
    let width = rows[0].len();
    for (i, row) in rows.iter().enumerate() {
        if row.len() != width {
            return Err(format!(
                "cannot read {}: row {} has {} cells, expected {width}",
                csv.display(),
                i + 1,
                row.len()
            ));
        }
    }
    let content = Content {
        blocks: vec![content::Block::Table(content::Table {
            rows: rows
                .iter()
                .map(|row| content::Row {
                    cells: row
                        .iter()
                        .map(|text| {
                            content::Cell::new(vec![content::Block::Paragraph {
                                style: None,
                                align: None,
                                content: vec![content::Inline::text(text)],
                            }])
                        })
                        .collect(),
                })
                .collect(),
            header_rows: 0,
            first_column: false,
            style: table.style.clone(),
            widths: None,
            caption: None,
            label: None,
        })],
        table_styles: Vec::new(),
    };
    let opts = WriteOptions::for_content(&content, None);
    doc.insert(Place::Replace(table.node), &content.blocks, &opts)
        .map_err(|e| format!("{}: {e}", input.display()))?;
    doc.save(output).map_err(|e| e.to_string())?;
    println!(
        "{}: replaced table {target} ({} × {})",
        output.display(),
        rows.len(),
        width
    );
    Ok(true)
}

/// The file name for a package part: its last segment, when that is a
/// single safe file name on the host. The package layer already refuses
/// `.` and `..` segments, backslashes and control characters at open;
/// colons stay possible there and would misbehave on Windows, where a
/// drive-prefixed name makes `Path::join` discard the output directory.
/// Windows also reserves device names with or without an extension and
/// normalizes trailing spaces and dots, so those are refused too.
fn safe_name(part: &str) -> Option<String> {
    let base = part.rsplit('/').next()?;
    if base.is_empty() || base == "." || base == ".." {
        return None;
    }
    if base.contains(['/', '\\', ':', '\0']) {
        return None;
    }
    if base.ends_with(' ') || base.ends_with('.') {
        return None;
    }
    let stem = base.split('.').next().unwrap_or(base);
    // Windows ignores spaces before the extension when matching.
    let stem = stem.trim_end_matches(' ');
    if is_windows_device_name(stem) {
        return None;
    }
    Some(base.to_string())
}

/// True for the basenames Windows reserves, with or without an
/// extension, compared without case. That includes the superscript
/// `COM¹`–`COM³` and `LPT¹`–`LPT³`, which `to_ascii_uppercase` leaves
/// intact for an exact match.
fn is_windows_device_name(stem: &str) -> bool {
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "COM¹"
            | "COM²"
            | "COM³"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
            | "LPT¹"
            | "LPT²"
            | "LPT³"
    )
}

/// A distinct file name with the same extension: `photo-2.png` for the
/// second `photo.png`.
fn numbered_name(base: &str, n: u32) -> String {
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
            format!("{stem}-{n}.{ext}")
        }
        _ => format!("{base}-{n}"),
    }
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
            Command::Images { file, dir } => images(file, dir),
            Command::Tables { file, dir, format } => tables(file, dir, *format),
            Command::Table {
                input,
                output,
                target,
                csv,
            } => replace_table(input, output, target, csv),
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

#[cfg(test)]
mod tests {
    use super::{numbered_name, safe_name};
    use std::collections::HashSet;

    #[test]
    fn file_names_stay_in_the_output_directory() {
        assert_eq!(
            safe_name("word/media/image1.png").as_deref(),
            Some("image1.png")
        );
        assert_eq!(safe_name("image.png").as_deref(), Some("image.png"));
        assert_eq!(safe_name("a/b").as_deref(), Some("b"));
        assert_eq!(safe_name("COM10.png").as_deref(), Some("COM10.png"));
        assert_eq!(safe_name("LPT10.png").as_deref(), Some("LPT10.png"));
        assert_eq!(safe_name("console.png").as_deref(), Some("console.png"));
        assert_eq!(safe_name(".NUL").as_deref(), Some(".NUL"));
        for bad in [
            "",
            "word/media/",
            ".",
            "..",
            "word/media/a:b.png",
            "a\\b.png",
            "photo.png ",
            "photo.",
            "NUL.png",
            "com1.png",
            "LPT9.dat",
            "com¹.png",
            "LPT³.dat",
            "NUL .png",
        ] {
            assert_eq!(safe_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn repeated_names_gain_a_number_before_the_extension() {
        assert_eq!(numbered_name("image1.png", 2), "image1-2.png");
        assert_eq!(numbered_name("no-ext", 3), "no-ext-3");
        assert_eq!(numbered_name(".png", 2), ".png-2");
        // The loop from `images`: first use keeps the base name.
        let mut used = HashSet::new();
        let mut next = |base: &str| {
            let mut name = base.to_string();
            let mut n = 2;
            while !used.insert(name.clone()) {
                name = numbered_name(base, n);
                n += 1;
            }
            name
        };
        assert_eq!(next("image1.png"), "image1.png");
        assert_eq!(next("image1.png"), "image1-2.png");
        assert_eq!(next("image1.png"), "image1-3.png");
    }
}

#[cfg(test)]
mod seq_tests {
    use super::is_table_seq;

    fn field(code: &str) -> nebu_docx::Field {
        let kind = code
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        nebu_docx::Field {
            index: 0,
            kind,
            code: code.to_string(),
            result: String::new(),
            simple: true,
            parent: None,
            location: nebu_docx::Location {
                story: nebu_docx::StoryKind::Main,
                part: String::new(),
                block: None,
            },
        }
    }

    #[test]
    fn table_sequences_match_in_any_case() {
        for code in [
            "SEQ Table",
            "SEQ Table \\* ARABIC",
            "SEQ Table \\c",
            "SEQ table",
            "seq table",
        ] {
            assert!(is_table_seq(&field(code)), "{code}");
        }
        for code in ["SEQ", "SEQ Figure", "REF _Ref_tab-sites \\h", "SEQ Tables"] {
            assert!(!is_table_seq(&field(code)), "{code}");
        }
    }
}
