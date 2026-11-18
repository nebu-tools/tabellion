//! The writer: copies what did not change, writes what did.
//!
//! The walk carries the namespace bindings in effect. A parsed node is
//! copied as written when nothing in it changed and the bindings it was
//! parsed under still hold where it now is. Otherwise its markup is written
//! from the tree, with prefixes chosen from the bindings in effect, and a
//! declaration added where a namespace has no prefix yet.
//!
//! Character data from different nodes can meet in the output where they
//! did not meet in the source: an edit puts new text next to old text, or
//! removes what stood between two texts. Each piece is well-formed alone, but
//! two pieces can join into `]]>`, which text may not contain, or into a
//! carriage return and a line feed, which a reader takes for one line end.
//! [`Run`] watches every boundary and escapes the one character that would
//! join them, in new text where it can and in source text where it must.

use std::borrow::Cow;

use super::chars::{decode, escape_attribute, escape_text};
use super::parse::{name_end, source_attrs};
use super::{
    AttrValue, CHANGED, DOCUMENT, Data, Element, Ix, Node, ScopeId, Tree, XML_NS, XMLNS_URI,
};

/// The namespace bindings in effect at a point of the output.
struct Env<'t> {
    bindings: Vec<(Cow<'t, str>, Option<Cow<'t, str>>)>,
    /// Set when the bindings are exactly those of this parsed scope, so
    /// that a node parsed in it can be copied without comparing bindings.
    fast: Option<ScopeId>,
}

impl<'t> Env<'t> {
    fn lookup(&self, prefix: &str) -> Option<&str> {
        if prefix == "xml" {
            return Some(XML_NS);
        }
        self.bindings
            .iter()
            .rev()
            .find(|(p, _)| *p == prefix)
            .and_then(|(_, u)| u.as_deref())
    }

    /// A prefix bound to `uri` and not hidden by a later binding.
    fn prefix_for(&self, uri: &str, allow_default: bool) -> Option<String> {
        self.bindings
            .iter()
            .rev()
            .filter(|(p, u)| u.as_deref() == Some(uri) && (allow_default || !p.is_empty()))
            .find(|(p, _)| self.lookup(p) == Some(uri))
            .map(|(p, _)| p.to_string())
    }

    /// True if every binding a node parsed in `ctx` relies on holds here.
    fn compatible(&self, t: &'t Tree, ctx: ScopeId) -> bool {
        if self.fast == Some(ctx) {
            return true;
        }
        let mut seen: Vec<&str> = vec![];
        let mut scope = Some(ctx);
        while let Some(id) = scope {
            let s = &t.scopes[id as usize];
            for (p, u) in &s.bindings {
                if seen.contains(&&**p) {
                    continue;
                }
                seen.push(p);
                if self.lookup(p) != u.map(|u| &*t.uris[u as usize]) {
                    return false;
                }
            }
            scope = s.parent;
        }
        seen.contains(&"") || self.lookup("").is_none()
    }

    fn push(&mut self, prefix: Cow<'t, str>, uri: Option<Cow<'t, str>>) {
        self.bindings.push((prefix, uri));
    }

    /// A prefix that nothing binds here and the element does not declare.
    fn free_prefix(&self, own: &[&str]) -> String {
        (0..)
            .map(|n| format!("ns{n}"))
            .find(|p| self.lookup(p).is_none() && !own.contains(&p.as_str()))
            .unwrap_or_default()
    }
}

enum Frame<'t> {
    Enter(Ix),
    Close {
        tag: Cow<'t, str>,
        env_len: usize,
        fast: Option<ScopeId>,
    },
}

/// Character data at the end of the output.
#[derive(Default)]
struct Run {
    /// Where the last piece of new text starts in the output, while it is
    /// the last thing written.
    new_from: Option<usize>,
}

impl Run {
    /// Writes text from the source, still escaped as it was.
    fn source(&mut self, raw: &str, out: &mut String) {
        let mut raw = raw;
        let joins_cdata_end = (raw.starts_with('>') && out.ends_with("]]"))
            || (raw.starts_with("]>") && out.ends_with(']'));
        if joins_cdata_end {
            match self.new_from {
                // The last `]` is new text: escape it there.
                Some(k) if out.len() > k => {
                    out.pop();
                    out.push_str("&#93;");
                }
                // Both sides are source text: escape the `>`.
                _ => {
                    let (head, tail) = raw.split_at(raw.find('>').unwrap_or(0) + 1);
                    out.push_str(&head[..head.len() - 1]);
                    out.push_str("&gt;");
                    raw = tail;
                }
            }
        }
        if out.ends_with('\r') && raw.starts_with('\n') {
            out.push_str("&#10;");
            raw = &raw[1..];
        }
        out.push_str(raw);
        self.new_from = None;
    }

    /// Writes new text. It escapes `>` and carriage returns itself, so the
    /// only join left is a line feed after a source carriage return.
    fn fresh(&mut self, text: &str, out: &mut String) {
        let mut text = text;
        let start = out.len();
        if out.ends_with('\r') && text.starts_with('\n') {
            out.push_str("&#10;");
            text = &text[1..];
        }
        escape_text(text, out);
        self.new_from = Some(start);
    }

    /// Writes markup, which ends any run of character data.
    fn markup(&mut self, s: &str, out: &mut String) {
        out.push_str(s);
        self.new_from = None;
    }
}

pub(super) fn write(t: &Tree) -> String {
    let mut out = String::with_capacity(t.bufs.first().map_or(1024, |b| b.len() + 1024));
    let mut env = Env {
        bindings: vec![],
        fast: Some(0),
    };
    let mut run = Run::default();
    let mut stack: Vec<Frame<'_>> = t
        .n(DOCUMENT)
        .children
        .iter()
        .rev()
        .map(|&c| Frame::Enter(c))
        .collect();
    while let Some(frame) = stack.pop() {
        let id = match frame {
            Frame::Close { tag, env_len, fast } => {
                run.markup(&tag, &mut out);
                env.bindings.truncate(env_len);
                env.fast = fast;
                continue;
            }
            Frame::Enter(id) => id,
        };
        let n = t.n(id);
        match &n.data {
            Data::Element(e) => {
                let in_scope = e.ctx.is_some_and(|c| env.compatible(t, c));
                if n.flags == 0
                    && in_scope
                    && let Some(span) = n.span
                {
                    run.markup(t.src(span), &mut out);
                    continue;
                }
                run.markup("", &mut out);
                let (env_len, fast) = (env.bindings.len(), env.fast);
                match start_tag(t, n, e, in_scope, &mut env, &mut out) {
                    Some(tag) => {
                        stack.push(Frame::Close { tag, env_len, fast });
                        stack.extend(n.children.iter().rev().map(|&c| Frame::Enter(c)));
                    }
                    None => {
                        env.bindings.truncate(env_len);
                        env.fast = fast;
                    }
                }
            }
            Data::Text(Some(s)) => run.fresh(s, &mut out),
            Data::Text(None) => {
                if let Some(span) = n.span {
                    run.source(t.src(span), &mut out);
                }
            }
            _ => {
                if let Some(span) = n.span {
                    run.markup(t.src(span), &mut out);
                }
            }
        }
    }
    out
}

/// An attribute or declaration as the writer sees it, from the tree or from
/// the source tag.
struct Item<'t> {
    /// For a declaration: the prefix it binds and the namespace.
    decl: Option<(&'t str, Option<Cow<'t, str>>)>,
    prefix: &'t str,
    local: &'t str,
    ns: Option<&'t str>,
    /// The value still escaped as in the source, or as plain text.
    value: Result<&'t str, &'t str>,
    /// The attribute as written, while its value is unchanged.
    raw: Option<&'t str>,
    quote: u8,
}

fn items<'t>(t: &'t Tree, e: &'t Element) -> Vec<Item<'t>> {
    let decl_of = |prefix: &'t str, local: &'t str, value: Cow<'t, str>| {
        let bound = if prefix == "xmlns" { local } else { "" };
        (bound, (!value.is_empty()).then_some(value))
    };
    match &e.attrs {
        Some(attrs) => attrs
            .iter()
            .map(|a| {
                let n = &t.names[a.name as usize];
                let (value, raw) = match &a.value {
                    AttrValue::Source(s) => (Ok(t.src(*s)), a.raw.map(|r| t.src(r))),
                    AttrValue::Owned(v) => (Err(&**v), None),
                };
                let decl = (n.ns == Some(XMLNS_URI)).then(|| {
                    let v = match value {
                        Ok(s) => decode(s, true),
                        Err(s) => Cow::Borrowed(s),
                    };
                    decl_of(&n.prefix, &n.local, v)
                });
                Item {
                    decl,
                    prefix: &n.prefix,
                    local: &n.local,
                    ns: n.ns.map(|u| &*t.uris[u as usize]),
                    value,
                    raw,
                    quote: a.quote,
                }
            })
            .collect(),
        None => {
            let (Some(open), Some(scope)) = (e.open, e.scope) else {
                return vec![];
            };
            let tag = t.src(open);
            source_attrs(tag)
                .map(|a| {
                    let qname = &tag[a.name.clone()];
                    let (prefix, local) = qname.split_once(':').unwrap_or(("", qname));
                    let value = &tag[a.value.clone()];
                    let is_decl = prefix == "xmlns" || (prefix.is_empty() && local == "xmlns");
                    let ns = if is_decl || prefix.is_empty() {
                        None
                    } else {
                        t.lookup(scope, prefix)
                            .flatten()
                            .map(|u| &*t.uris[u as usize])
                    };
                    Item {
                        decl: is_decl.then(|| decl_of(prefix, local, decode(value, true))),
                        prefix,
                        local,
                        ns,
                        value: Ok(value),
                        raw: Some(&tag[a.raw.clone()]),
                        quote: a.quote,
                    }
                })
                .collect()
        }
    }
}

fn qualified(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_string()
    } else {
        format!("{prefix}:{local}")
    }
}

/// Writes the start tag and returns the end tag, or `None` for an empty
/// element written as `<…/>`.
fn start_tag<'t>(
    t: &'t Tree,
    n: &'t Node,
    e: &'t Element,
    in_scope: bool,
    env: &mut Env<'t>,
    out: &mut String,
) -> Option<Cow<'t, str>> {
    let has_children = !n.children.is_empty();
    let source = if in_scope {
        e.open.map(|s| t.src(s))
    } else {
        None
    };
    let fast_before = env.fast;
    let len_before = env.bindings.len();

    // The start tag as written, when only the children changed.
    if let Some(tag) = source
        && n.flags & CHANGED == 0
    {
        if e.scope != e.ctx
            && let Some(scope) = e.scope
        {
            for (p, u) in &t.scopes[scope as usize].bindings {
                env.push(
                    Cow::Borrowed(p),
                    u.map(|u| Cow::Borrowed(&*t.uris[u as usize])),
                );
            }
        }
        env.fast = if fast_before == e.ctx { e.scope } else { None };
        let self_closed = tag.ends_with("/>");
        let end_tag = || Cow::Owned(format!("</{}>", &tag[1..name_end(tag, 1)]));
        if self_closed && has_children {
            out.push_str(&tag[..tag.len() - 2]);
            out.push('>');
            return Some(end_tag());
        }
        out.push_str(tag);
        if self_closed {
            return None;
        }
        return Some(e.close.map_or_else(end_tag, |c| Cow::Borrowed(t.src(c))));
    }

    let items = items(t, e);
    for it in &items {
        if let Some((p, u)) = &it.decl {
            env.push(Cow::Borrowed(p), u.clone());
        }
    }
    // Prefixes the element declares itself: never free to declare again.
    let own: Vec<&str> = items
        .iter()
        .filter_map(|it| it.decl.as_ref().map(|(p, _)| *p))
        .collect();
    let mut extra: Vec<(String, Option<String>)> = vec![];
    let name = &t.names[e.name as usize];
    let qname: Cow<'t, str> = match source {
        Some(tag) if !e.renamed => Cow::Borrowed(&tag[1..name_end(tag, 1)]),
        _ => Cow::Owned(element_name(
            name.ns.map(|u| &*t.uris[u as usize]),
            &name.prefix,
            &name.local,
            &own,
            env,
            &mut extra,
        )),
    };
    let mut body = String::new();
    for it in &items {
        let kept = source.is_some() && it.value.is_ok();
        match (&it.decl, it.raw) {
            (_, Some(raw)) if kept => body.push_str(raw),
            (Some((p, u)), _) => {
                body.push_str(if p.is_empty() { " xmlns" } else { " xmlns:" });
                body.push_str(p);
                body.push_str("=\"");
                escape_attribute(u.as_deref().unwrap_or(""), '"', &mut body);
                body.push('"');
            }
            (None, _) => {
                let q = attribute_name(it.ns, it.prefix, it.local, &own, env, &mut extra);
                let quote = it.quote as char;
                body.push(' ');
                body.push_str(&q);
                body.push('=');
                body.push(quote);
                match it.value {
                    Ok(escaped) if !escaped.contains(quote) => body.push_str(escaped),
                    Ok(escaped) => escape_attribute(&decode(escaped, true), quote, &mut body),
                    Err(plain) => escape_attribute(plain, quote, &mut body),
                }
                body.push(quote);
            }
        }
    }
    out.push('<');
    out.push_str(&qname);
    for (p, u) in &extra {
        out.push_str(if p.is_empty() { " xmlns" } else { " xmlns:" });
        out.push_str(p);
        out.push_str("=\"");
        escape_attribute(u.as_deref().unwrap_or(""), '"', out);
        out.push('"');
    }
    out.push_str(&body);
    if let Some(tag) = source {
        let inner_end = tag.len() - if tag.ends_with("/>") { 2 } else { 1 };
        let last = source_attrs(tag)
            .last()
            .map_or(name_end(tag, 1), |a| a.raw.end);
        out.push_str(&tag[last.min(inner_end)..inner_end]);
    }
    env.fast = if source.is_some() && fast_before == e.ctx && extra.is_empty() {
        e.scope
    } else if env.bindings.len() == len_before {
        fast_before
    } else {
        None
    };
    if !has_children {
        out.push_str("/>");
        return None;
    }
    out.push('>');
    Some(match (source, e.close) {
        (Some(_), Some(c)) if !e.renamed => Cow::Borrowed(t.src(c)),
        _ => Cow::Owned(format!("</{qname}>")),
    })
}

/// The qualified name for an element, declaring its namespace if needed.
fn element_name(
    ns: Option<&str>,
    hint: &str,
    local: &str,
    own: &[&str],
    env: &mut Env<'_>,
    extra: &mut Vec<(String, Option<String>)>,
) -> String {
    let Some(uri) = ns else {
        if env.lookup("").is_some() {
            extra.push((String::new(), None));
            env.push(Cow::Borrowed(""), None);
        }
        return local.to_string();
    };
    if env.lookup(hint) == Some(uri) {
        return qualified(hint, local);
    }
    if let Some(p) = env.prefix_for(uri, true) {
        return qualified(&p, local);
    }
    let p = if env.lookup(hint).is_none() && !own.contains(&hint) {
        hint.to_string()
    } else {
        env.free_prefix(own)
    };
    declare(&p, uri, env, extra);
    qualified(&p, local)
}

/// The qualified name for an attribute, declaring its namespace if needed.
fn attribute_name(
    ns: Option<&str>,
    hint: &str,
    local: &str,
    own: &[&str],
    env: &mut Env<'_>,
    extra: &mut Vec<(String, Option<String>)>,
) -> String {
    let Some(uri) = ns else {
        return local.to_string();
    };
    if !hint.is_empty() && env.lookup(hint) == Some(uri) {
        return qualified(hint, local);
    }
    if let Some(p) = env.prefix_for(uri, false) {
        return qualified(&p, local);
    }
    let p = if !hint.is_empty() && env.lookup(hint).is_none() && !own.contains(&hint) {
        hint.to_string()
    } else {
        env.free_prefix(own)
    };
    declare(&p, uri, env, extra);
    qualified(&p, local)
}

fn declare(prefix: &str, uri: &str, env: &mut Env<'_>, extra: &mut Vec<(String, Option<String>)>) {
    extra.push((prefix.to_string(), Some(uri.to_string())));
    env.push(
        Cow::Owned(prefix.to_string()),
        Some(Cow::Owned(uri.to_string())),
    );
}
