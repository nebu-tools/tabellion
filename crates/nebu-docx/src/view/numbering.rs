//! The numbering part as read: list definitions and the instances that
//! paragraphs refer to. Computing the number Word shows is not done here.

use serde::Serialize;

use crate::Error;
use crate::xml::{NodeId, Tree};

use super::props::{child_val, on_off};
use super::walk::W;

/// The lists of a document.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Numbering {
    /// The definitions (`w:abstractNum`).
    pub definitions: Vec<NumberingDefinition>,
    /// The instances paragraphs refer to (`w:num`).
    pub instances: Vec<NumberingInstance>,
}

/// A list definition.
#[derive(Debug, Clone, Serialize)]
pub struct NumberingDefinition {
    /// Its id (`w:abstractNumId`).
    pub id: String,
    /// Its name, if it has one.
    pub name: Option<String>,
    /// `single_level`, `multilevel` or `hybrid_multilevel`.
    pub kind: Option<String>,
    /// The numbering style this definition is (`w:styleLink`).
    pub style: Option<String>,
    /// The numbering style that holds the definition (`w:numStyleLink`).
    pub uses_style: Option<String>,
    /// Its levels.
    pub levels: Vec<NumberingLevel>,
}

/// One level of a list definition.
#[derive(Debug, Clone, Serialize)]
pub struct NumberingLevel {
    /// The level, from 0.
    pub level: u8,
    /// The first number.
    pub start: Option<i64>,
    /// The number format: `decimal`, `lowerRoman`, `bullet` and so on.
    pub format: Option<String>,
    /// The text of the number, with `%1` for the number of level 1.
    pub text: Option<String>,
    /// The paragraph style tied to the level.
    pub style: Option<String>,
    /// The level after which numbering starts again.
    pub restart_after: Option<u8>,
    /// Numbers of all levels in decimal (`w:isLgl`).
    pub legal: bool,
}

/// A list instance.
#[derive(Debug, Clone, Serialize)]
pub struct NumberingInstance {
    /// Its id (`w:numId`), which paragraphs use.
    pub id: String,
    /// The definition it uses.
    pub definition: Option<String>,
    /// Levels it changes.
    pub overrides: Vec<LevelOverride>,
}

/// A change an instance makes to one level of its definition.
#[derive(Debug, Clone, Serialize)]
pub struct LevelOverride {
    /// The level, from 0.
    pub level: u8,
    /// A new first number.
    pub start: Option<i64>,
    /// A new definition of the level.
    pub definition: Option<NumberingLevel>,
}

pub(crate) fn read(tree: Option<&Tree>) -> Result<Numbering, Error> {
    let mut out = Numbering::default();
    let Some(t) = tree else { return Ok(out) };
    for el in t.elements(t.root_element())? {
        if t.is(el, Some(W), "abstractNum")? {
            let mut levels = vec![];
            for l in t.elements(el)? {
                if t.is(l, Some(W), "lvl")? {
                    levels.push(level(t, l)?);
                }
            }
            out.definitions.push(NumberingDefinition {
                id: attr(t, el, "abstractNumId")?.unwrap_or_default(),
                name: child_val(t, el, "name")?,
                kind: child_val(t, el, "multiLevelType")?.map(|k| match k.as_str() {
                    "singleLevel" => "single_level".into(),
                    "multilevel" => "multilevel".into(),
                    "hybridMultilevel" => "hybrid_multilevel".into(),
                    _ => k,
                }),
                style: child_val(t, el, "styleLink")?,
                uses_style: child_val(t, el, "numStyleLink")?,
                levels,
            });
        } else if t.is(el, Some(W), "num")? {
            let mut overrides = vec![];
            for o in t.elements(el)? {
                if !t.is(o, Some(W), "lvlOverride")? {
                    continue;
                }
                overrides.push(LevelOverride {
                    level: attr(t, o, "ilvl")?
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0),
                    start: child_val(t, o, "startOverride")?.and_then(|v| v.parse().ok()),
                    definition: match t.child(o, Some(W), "lvl")? {
                        Some(l) => Some(level(t, l)?),
                        None => None,
                    },
                });
            }
            out.instances.push(NumberingInstance {
                id: attr(t, el, "numId")?.unwrap_or_default(),
                definition: child_val(t, el, "abstractNumId")?,
                overrides,
            });
        }
    }
    Ok(out)
}

fn attr(t: &Tree, n: NodeId, local: &str) -> Result<Option<String>, Error> {
    Ok(t.attribute(n, Some(W), local)?.map(|v| v.into_owned()))
}

fn level(t: &Tree, l: NodeId) -> Result<NumberingLevel, Error> {
    Ok(NumberingLevel {
        level: attr(t, l, "ilvl")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        start: child_val(t, l, "start")?.and_then(|v| v.parse().ok()),
        format: child_val(t, l, "numFmt")?,
        text: child_val(t, l, "lvlText")?,
        style: child_val(t, l, "pStyle")?,
        restart_after: child_val(t, l, "lvlRestart")?.and_then(|v| v.parse().ok()),
        legal: match t.child(l, Some(W), "isLgl")? {
            Some(c) => on_off(t, c)?,
            None => false,
        },
    })
}
