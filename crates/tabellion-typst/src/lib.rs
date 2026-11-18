//! Typst as an authoring path for Word.
//!
//! A Typst fragment is evaluated in-process with the `typst` crates. The
//! evaluated content tree, not a laid-out page, is walked and mapped to the
//! engine's content model ([`nebu_docx::content`]), which the engine then
//! writes in the target document's own styles. Elements with a native Word
//! form become that form. An element without one is laid out by Typst and
//! embedded as a picture, and every such case is reported as a warning, so
//! nothing disappears without notice.

mod math;
mod value;
mod walk;
mod world;

use std::collections::HashMap;
use std::path::PathBuf;

pub use nebu_docx::content::Block;
pub use nebu_docx::table_style::TableStyleDef;

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Directory that `image("…")`, `json("…")` and imports resolve against.
    pub root: PathBuf,
    /// `sys.inputs`.
    pub inputs: Vec<(String, String)>,
    /// Files that exist only in memory, by path relative to the root, for
    /// example a data frame serialised as JSON by R.
    pub files: HashMap<String, Vec<u8>>,
    /// Resolution for elements that become pictures, in pixels per point.
    pub raster_scale: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Fragment {
    pub blocks: Vec<Block>,
    /// Table styles defined with `tabellion.table-style` in the fragment.
    pub table_styles: Vec<TableStyleDef>,
    /// Everything that did not map one to one: what it was and what became of it.
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The Typst helpers that fragments can import as `#import "@tabellion": *`
/// or that are prepended to a fragment.
pub const PRELUDE: &str = include_str!("../typst/tabellion.typ");

/// Evaluates Typst source and maps its content to Word content.
pub fn convert(source: &str, opts: &Options) -> Result<Fragment, Error> {
    let world = world::FragmentWorld::new(source, opts);
    let module = world.eval()?;
    let mut w = walk::Walker::new(&world, opts);
    w.run(module.content());
    Ok(w.finish())
}

/// Evaluates Typst source and returns the value bound to `name` at its top
/// level as JSON, e.g. a table style written as a dictionary.
pub fn binding(source: &str, name: &str, opts: &Options) -> Result<serde_json::Value, Error> {
    let world = world::FragmentWorld::new(source, opts);
    let module = world.eval()?;
    let v = module
        .scope()
        .get(name)
        .ok_or_else(|| Error(format!("the Typst source defines no {name:?}")))?;
    Ok(value::to_json(v.read()))
}

/// A table style from a Typst dictionary bound to `name`.
pub fn table_style(source: &str, name: &str, opts: &Options) -> Result<TableStyleDef, Error> {
    let v = binding(source, name, opts)?;
    serde_json::from_value(v).map_err(|e| Error(format!("{name} is not a table style: {e}")))
}
