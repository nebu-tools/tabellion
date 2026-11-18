//! The `tabellion` command line. This is the spike's subset of the design in
//! DESIGN.md: inspect, insert, replace, accept/reject, roundtrip, bench.

use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand};
use nebu_docx::document::{BlockKind, Locator};
use nebu_docx::{Block, Document, Place, Revision, Roles, WriteOptions};

#[derive(Parser)]
#[command(name = "tabellion", version, about = "Inspect, fill, convert and edit Word documents")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the document's blocks, styles or revisions.
    Inspect {
        file: PathBuf,
        /// outline, blocks, styles, revisions
        #[arg(default_value = "outline")]
        what: String,
        #[arg(long)]
        json: bool,
    },
    /// Insert content at a place: `{{name}}`, bookmark:NAME, heading:TEXT, block:ID, text:…, end.
    Insert {
        file: PathBuf,
        out: PathBuf,
        #[arg(long)]
        at: String,
        /// Typst source file.
        #[arg(long, conflicts_with = "json")]
        typst: Option<PathBuf>,
        /// Content model as JSON (a list of blocks).
        #[arg(long)]
        json: Option<PathBuf>,
        /// before, after or replace (default: replace for placeholders, after otherwise).
        #[arg(long)]
        mode: Option<String>,
        /// Track the insertion under this author.
        #[arg(long)]
        author: Option<String>,
    },
    /// Replace text inside one paragraph, as a tracked change when --author is given.
    Replace {
        file: PathBuf,
        out: PathBuf,
        #[arg(long)]
        find: String,
        #[arg(long = "with")]
        with: String,
        #[arg(long)]
        author: Option<String>,
        /// Limit to one block (see `inspect blocks`).
        #[arg(long)]
        block: Option<String>,
    },
    /// Accept all revisions, or those of one author.
    Accept { file: PathBuf, out: PathBuf, #[arg(long)] author: Option<String> },
    /// Reject all revisions, or those of one author.
    Reject { file: PathBuf, out: PathBuf, #[arg(long)] author: Option<String> },
    /// Read and write without changes; report which zip entries are identical.
    Roundtrip { file: PathBuf, out: PathBuf },
    /// Time open, parse, write and one tracked replacement.
    Bench {
        file: PathBuf,
        #[arg(long)]
        find: String,
        #[arg(long = "with")]
        with: String,
        #[arg(long, default_value = "5")]
        runs: usize,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Only look in this block.
        #[arg(long)]
        block: Option<String>,
    },
}

type R<T> = Result<T, Box<dyn std::error::Error>>;

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("tabellion: {e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> R<()> {
    match cli.cmd {
        Cmd::Inspect { file, what, json } => inspect(&file, &what, json),
        Cmd::Insert { file, out, at, typst, json, mode, author } => {
            let mut doc = Document::open(&file)?;
            let roles = Roles::detect(doc.styles()?);
            let (blocks, styles, warnings) = match (typst, json) {
                (Some(t), _) => {
                    let src = std::fs::read_to_string(&t)?;
                    let opts = tabellion_typst::Options { root: t.parent().unwrap_or(std::path::Path::new(".")).to_path_buf(), ..Default::default() };
                    let f = tabellion_typst::convert(&src, &opts)?;
                    (f.blocks, f.table_styles, f.warnings)
                }
                (None, Some(j)) => (serde_json::from_str::<Vec<Block>>(&std::fs::read_to_string(j)?)?, vec![], vec![]),
                _ => return Err("give --typst or --json".into()),
            };
            for w in &warnings {
                eprintln!("warning: {w}");
            }
            let place = place(&mut doc, &at, mode.as_deref())?;
            let o = WriteOptions { roles, track: author.map(|a| Revision::now(&a)), table_styles: styles };
            doc.insert(place, &blocks, &o)?;
            doc.save(&out)?;
            Ok(())
        }
        Cmd::Replace { file, out, find, with, author, block } => {
            let mut doc = Document::open(&file)?;
            let rev = author.map(|a| Revision::now(&a));
            let targets: Vec<_> = match block {
                Some(b) => vec![doc.locate(&Locator::Block(b))?],
                None => doc.blocks()?,
            };
            let mut done = false;
            for b in targets {
                let paras: Vec<_> = {
                    let t = doc.tree();
                    std::iter::once(b.node).chain(t.descendants(b.node)).filter(|&n| t.is(n, "w:p")).collect()
                };
                for p in paras {
                    if nebu_docx::revise::replace_in_paragraph(&mut doc, p, &find, &with, rev.as_ref())? {
                        done = true;
                        break;
                    }
                }
                if done {
                    break;
                }
            }
            if !done {
                return Err(format!("{find:?} does not occur in one paragraph").into());
            }
            nebu_docx::fields::update(&mut doc)?;
            doc.save(&out)?;
            Ok(())
        }
        Cmd::Accept { file, out, author } => settle(&file, &out, true, author.as_deref()),
        Cmd::Reject { file, out, author } => settle(&file, &out, false, author.as_deref()),
        Cmd::Roundtrip { file, out } => {
            let mut doc = Document::open(&file)?;
            doc.blocks()?;
            doc.save(&out)?;
            let (same, total) = compare_entries(&file, &out)?;
            println!("{same} of {total} zip entries identical (compressed bytes and CRC)");
            Ok(())
        }
        Cmd::Bench { file, find, with, runs, out, block } => bench(&file, &find, &with, runs, out, block),
    }
}

fn place(doc: &mut Document, at: &str, mode: Option<&str>) -> R<Place> {
    if at == "end" {
        return Ok(Place::End);
    }
    let loc: Locator = at.parse()?;
    let b = doc.locate(&loc)?;
    let default = if matches!(loc, Locator::Placeholder(_)) { "replace" } else { "after" };
    Ok(match mode.unwrap_or(default) {
        "before" => Place::Before(b.node),
        "after" => Place::After(b.node),
        "replace" => Place::Replace(b.node),
        m => return Err(format!("unknown mode {m}").into()),
    })
}

fn settle(file: &PathBuf, out: &PathBuf, accept: bool, author: Option<&str>) -> R<()> {
    let mut doc = Document::open(file)?;
    let body = doc.body();
    let n = nebu_docx::revise::settle(doc.tree(), body, accept, author);
    nebu_docx::fields::update(&mut doc)?;
    doc.save(out)?;
    eprintln!("{} {n} revisions", if accept { "accepted" } else { "rejected" });
    Ok(())
}

fn inspect(file: &PathBuf, what: &str, json: bool) -> R<()> {
    let mut doc = Document::open(file)?;
    match what {
        "styles" => {
            let s = doc.styles()?.list.clone();
            if json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                for x in s {
                    println!("{:<10} {:<28} {}", format!("{:?}", x.kind).to_lowercase(), x.id, x.name);
                }
            }
        }
        "fields" => {
            for (code, result) in nebu_docx::fields::list(&mut doc) {
                if json {
                    println!("{}", serde_json::json!({"code": code, "result": result}));
                } else {
                    println!("{code:<40} {result}");
                }
            }
        }
        "revisions" => {
            let body = doc.body();
            let t = doc.tree();
            let mut v = vec![];
            for d in t.descendants(body) {
                if matches!(t.name(d), "w:ins" | "w:del") {
                    v.push(serde_json::json!({
                        "kind": &t.name(d)[2..], "author": t.attr(d, "w:author"), "date": t.attr(d, "w:date"),
                        "text": nebu_docx::document::visible_text(t, d) + &t.descendants(d).filter(|&x| t.is(x, "w:delText")).map(|x| t.inner_text(x)).collect::<String>(),
                    }));
                }
            }
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        _ => {
            let blocks = doc.blocks()?;
            let blocks: Vec<_> = if what == "outline" { blocks.into_iter().filter(|b| b.heading_level.is_some() || b.kind == BlockKind::Table).collect() } else { blocks };
            if json {
                println!("{}", serde_json::to_string_pretty(&blocks)?);
            } else {
                for b in blocks {
                    let indent = "  ".repeat(b.heading_level.unwrap_or(1).saturating_sub(1) as usize);
                    let text: String = b.text.chars().take(70).collect::<String>().replace('\n', " ");
                    println!("{:<12} {indent}{}{}", b.id, if b.kind == BlockKind::Table { "[table] " } else { "" }, text);
                }
            }
        }
    }
    Ok(())
}

fn compare_entries(a: &PathBuf, b: &PathBuf) -> R<(usize, usize)> {
    let mut za = zip::ZipArchive::new(std::fs::File::open(a)?)?;
    let mut zb = zip::ZipArchive::new(std::fs::File::open(b)?)?;
    let mut same = 0;
    for i in 0..za.len() {
        let (name, crc, size) = {
            let f = za.by_index_raw(i)?;
            (f.name()?.to_string(), f.crc32(), f.compressed_size())
        };
        if let Ok(f) = zb.by_name(&name) {
            if f.crc32() == crc && f.compressed_size() == size {
                same += 1;
            }
        }
    }
    Ok((same, za.len()))
}

fn bench(file: &PathBuf, find: &str, with: &str, runs: usize, out: Option<PathBuf>, block: Option<String>) -> R<()> {
    let mut rows: Vec<[f64; 6]> = vec![];
    let tmp = std::env::temp_dir().join("tabellion-bench.docx");
    let mut fields_changed = 0;
    for _ in 0..runs {
        let t0 = Instant::now();
        let bytes = std::fs::read(file)?;
        let mut doc = Document::from_bytes(bytes)?;
        let t_open = t0.elapsed().as_secs_f64();
        let t1 = Instant::now();
        let blocks = doc.blocks()?;
        let t_parse = t1.elapsed().as_secs_f64();
        let blocks: Vec<_> = blocks.into_iter().filter(|b| block.as_ref().is_none_or(|id| &b.id == id)).collect();
        let t2 = Instant::now();
        let same = doc.to_bytes()?;
        std::fs::write(&tmp, &same)?;
        let t_write = t2.elapsed().as_secs_f64();
        let t3 = Instant::now();
        let rev = Revision::now("Bench");
        let mut done = false;
        for b in &blocks {
            let paras: Vec<_> = {
                let t = doc.tree();
                std::iter::once(b.node).chain(t.descendants(b.node)).filter(|&n| t.is(n, "w:p")).collect()
            };
            for p in paras {
                if nebu_docx::revise::replace_in_paragraph(&mut doc, p, find, with, Some(&rev))? {
                    done = true;
                    break;
                }
            }
            if done {
                break;
            }
        }
        if !done {
            return Err(format!("{find:?} not found").into());
        }
        let t_replace = t3.elapsed().as_secs_f64();
        let t4 = Instant::now();
        let n_fields = nebu_docx::fields::update(&mut doc)?;
        let t_fields = t4.elapsed().as_secs_f64();
        fields_changed = n_fields;
        let t5 = Instant::now();
        let bytes = doc.to_bytes()?;
        if let Some(o) = &out {
            std::fs::write(o, &bytes)?;
        }
        let t_save = t5.elapsed().as_secs_f64();
        rows.push([t_open, t_parse, t_write, t_replace, t_fields, t_save]);
    }
    let (same, total) = compare_entries(file, &tmp)?;
    let names = ["open (zip index)", "parse body + styles", "write unchanged", "tracked replace", "update fields", "write changed"];
    let med = |i: usize| {
        let mut v: Vec<f64> = rows.iter().map(|r| r[i]).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let mut total_ms = 0.0;
    for (i, n) in names.iter().enumerate() {
        println!("{n:<22} {:>9.1} ms", med(i) * 1000.0);
        total_ms += med(i) * 1000.0;
    }
    println!("{:<22} {:>9.1} ms (median of {runs})", "total", total_ms);
    println!("unchanged write: {same} of {total} zip entries identical; field results changed by the update: {fields_changed}");
    Ok(())
}
