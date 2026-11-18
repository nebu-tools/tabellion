//! A Typst world for one fragment: the fragment is `main.typ`, other files
//! come from the root directory or from memory, fonts are Typst's embedded
//! ones (they matter only for elements that become pictures).

use std::collections::HashMap;
use std::sync::OnceLock;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Dict, IntoValue, Module, Str};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

use crate::{Error, Options};

pub struct FragmentWorld {
    library: LazyHash<Library>,
    main: FileId,
    source: Source,
    root: std::path::PathBuf,
    files: HashMap<String, Bytes>,
    prelude: FileId,
}

fn fonts() -> &'static (LazyHash<FontBook>, Vec<Font>) {
    static F: OnceLock<(LazyHash<FontBook>, Vec<Font>)> = OnceLock::new();
    F.get_or_init(|| {
        let fonts: Vec<Font> = typst_kit::fonts::embedded().map(|(f, _)| f).collect();
        (LazyHash::new(FontBook::from_fonts(&fonts)), fonts)
    })
}

fn file_id(path: &str) -> FileId {
    FileId::new(RootedPath::new(VirtualRoot::Project, VirtualPath::new(path).expect("valid path")))
}

impl FragmentWorld {
    pub fn new(text: &str, opts: &Options) -> FragmentWorld {
        let mut inputs = Dict::new();
        for (k, v) in &opts.inputs {
            inputs.insert(Str::from(k.as_str()), v.as_str().into_value());
        }
        let main = file_id("/__fragment__.typ");
        let prelude = file_id("/@tabellion.typ");
        FragmentWorld {
            library: LazyHash::new(Library::builder().with_inputs(inputs).build()),
            main,
            source: Source::new(main, text.replace("\"@tabellion\"", "\"/@tabellion.typ\"")),
            root: opts.root.clone(),
            files: opts.files.iter().map(|(k, v)| (k.trim_start_matches('/').to_string(), Bytes::new(v.clone()))).collect(),
            prelude,
        }
    }

    pub fn eval(&self) -> Result<Module, Error> {
        use comemo::Track;
        let world: &dyn World = self;
        let mut sink = typst::engine::Sink::new();
        let traced = typst::engine::Traced::default();
        let route = typst::engine::Route::default();
        typst_eval::eval(world.track(), &self.library, traced.track(), sink.track_mut(), route.track(), &self.source)
            .map_err(|errs| Error(errs.iter().map(|e| self.describe(e)).collect::<Vec<_>>().join("\n")))
    }

    fn describe(&self, e: &typst::diag::SourceDiagnostic) -> String {
        use typst::WorldExt;
        match self.range(e.span) {
            Some(r) if r.end <= self.source.text().len() => {
                format!("{} (at `{}`)", e.message, self.source.text()[r].chars().take(60).collect::<String>())
            }
            _ => e.message.to_string(),
        }
    }

    /// The source text an element came from, if it came from the fragment.
    pub fn span_text(&self, span: typst::syntax::Span) -> Option<String> {
        if span.id() != Some(self.main) {
            return None;
        }
        use typst::WorldExt;
        let r = self.range(span)?;
        Some(self.source.text()[r].to_string())
    }

    #[allow(dead_code)]
    pub fn source_text(&self) -> &str {
        self.source.text()
    }
}

impl World for FragmentWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &fonts().0
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            return Ok(self.source.clone());
        }
        if id == self.prelude {
            return Ok(Source::new(id, crate::PRELUDE.to_string()));
        }
        let bytes = self.file(id)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| FileError::InvalidUtf8)?;
        Ok(Source::new(id, text.to_string()))
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        let rel = id.vpath().get_without_slash().to_string();
        if let Some(b) = self.files.get(&rel) {
            return Ok(b.clone());
        }
        let path = id.vpath().realize(&self.root).map_err(|_| FileError::AccessDenied)?;
        std::fs::read(&path).map(Bytes::new).map_err(|e| FileError::from_io(e, &path))
    }

    fn font(&self, index: usize) -> Option<Font> {
        fonts().1.get(index).cloned()
    }

    fn today(&self, _offset: Option<typst::foundations::Duration>) -> Option<Datetime> {
        None
    }
}

/// Compiles a standalone snippet and renders its first page to PNG.
/// Returns the PNG and the page size in points.
pub fn render_png(snippet: &str, opts: &Options) -> Result<(Vec<u8>, f64, f64), Error> {
    let world = FragmentWorld::new(snippet, opts);
    let doc: typst_layout::PagedDocument = typst::compile(&world)
        .output
        .map_err(|errs| Error(errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; ")))?;
    let page = doc.pages().first().ok_or_else(|| Error("nothing to render".into()))?;
    let scale = if opts.raster_scale > 0.0 { opts.raster_scale } else { 4.0 };
    let pix = typst_render::render(page, &typst_render::RenderOptions { pixel_per_pt: (scale as f64).into(), ..Default::default() });
    let png = pix.encode_png().map_err(|e| Error(e.to_string()))?;
    let size = page.frame.size();
    Ok((png, size.x.to_pt(), size.y.to_pt()))
}
