//! What the source actually contains, as opposed to what the document says
//! about it.
//!
//! Ply has always compared a declared design against *Cargo package*
//! dependencies. That answers "may this crate depend on that crate" and
//! nothing smaller, so a design restriction between two modules of one
//! crate has had no mechanism behind it. This module is the other half of
//! the comparison: a record of what the code really refers to, carrying
//! enough identity and provenance that a disagreement can be pointed at a
//! line rather than asserted.
//!
//! **Three rules hold everything here together.**
//!
//! A display name is not an identity. Two crates in one build can be
//! called `parser`, one type can have methods implemented three modules
//! away from its declaration, and a re-export gives an item a second path
//! without giving it a second home. Every identity below is therefore a
//! structured value, and none of them is a string a reader would recognise
//! from the source.
//!
//! A guess is never a fact. A reference whose destination could not be
//! resolved is recorded as unresolved, with the reason; it never becomes a
//! definite crossing, and it never quietly disappears either -- it leaves
//! the rule it might have broken *incompletely checked*, which is a
//! different answer from "no violation found".
//!
//! Everything is scoped to one build. "No forbidden reference" under one
//! feature set says nothing about another, so the configuration the facts
//! were gathered under travels with them.

use std::fmt;

/// The one build these observations describe.
///
/// Carried rather than assumed: the same source under a different feature
/// set is a different program, and a result that does not say which one it
/// looked at cannot be checked or reused.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BuildContext {
    /// Cargo package id, not the display name -- two packages can share a
    /// name.
    pub package: String,
    /// Which Cargo target within that package: the library, a binary, an
    /// integration test.
    pub target: String,
    /// Features Cargo reported as enabled for this invocation.
    pub features: Vec<String>,
    pub target_triple: String,
    /// What produced these observations, so a stale result can be told
    /// from a current one.
    pub analyzer: String,
}

impl BuildContext {
    /// A context for tests and for callers that have only one target in
    /// play. Real callers fill this from Cargo rather than composing it.
    pub fn simple(package: &str, target: &str) -> Self {
        Self {
            package: package.to_string(),
            target: target.to_string(),
            features: Vec::new(),
            target_triple: String::new(),
            analyzer: String::new(),
        }
    }
}

/// A module, identified by where it is declared rather than by a file
/// name.
///
/// `#[path]`, inline modules and `include!` all break the assumption that
/// a file's location gives its module path, so the path here is the chain
/// of `mod` declarations that actually reaches it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId {
    /// The crate this module lives in, by its identity in this build.
    pub krate: String,
    /// The `mod` names from the crate root down. Empty at the root itself.
    pub path: Vec<String>,
}

impl ModuleId {
    pub fn root(krate: &str) -> Self {
        Self {
            krate: krate.to_string(),
            path: Vec::new(),
        }
    }

    /// `crate::a::b`, written the way an anchor is written.
    pub fn parse(anchor: &str) -> Self {
        let mut parts = anchor.split("::").filter(|s| !s.is_empty());
        let krate = parts.next().unwrap_or_default().to_string();
        Self {
            krate,
            path: parts.map(|s| s.to_string()).collect(),
        }
    }

    /// Whether `self` is this module or one of its ancestors.
    ///
    /// **Ancestry, not string prefix.** `parse::rhythm` is not inside
    /// `parse::r`, and a check written with `starts_with` says it is.
    pub fn contains(&self, other: &ModuleId) -> bool {
        self.krate == other.krate
            && other.path.len() >= self.path.len()
            && self.path.iter().zip(&other.path).all(|(a, b)| a == b)
    }

    /// How deep this module sits, so the most specific owner can be
    /// picked without comparing text lengths.
    pub fn depth(&self) -> usize {
        self.path.len()
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.krate)?;
        for p in &self.path {
            write!(f, "::{p}")?;
        }
        Ok(())
    }
}

/// What kind of thing an item is. The distinction that matters here is
/// which ones can *hold* a reference and which ones only receive one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Function,
    /// A method, and the type it is implemented for.
    ///
    /// Its source owner is the module holding the `impl` block, which need
    /// not be the module declaring the type -- so the type is recorded
    /// beside it rather than standing in for it.
    Method {
        implemented_for: String,
    },
    Type,
    Const,
    Static,
    Module,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Visibility {
    Private,
    Crate,
    Public,
}

/// Where something is in the source, for a report a reader can act on.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

impl Span {
    pub fn at(file: &str, line: u32) -> Self {
        Self {
            file: file.to_string(),
            line,
            column: 0,
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

/// One named thing in the source.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId {
    /// The module that **declares** it. A re-export elsewhere adds a path,
    /// not a second declaration, so this does not move.
    pub module: ModuleId,
    pub name: String,
}

impl ItemId {
    pub fn new(module: ModuleId, name: &str) -> Self {
        Self {
            module,
            name: name.to_string(),
        }
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.module, self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRecord {
    pub id: ItemId,
    pub kind: ItemKind,
    pub visibility: Visibility,
    pub span: Span,
}

/// How one item reaches another.
///
/// The kinds are kept apart because they carry different weight. An import
/// establishes that a module is coupled to another; it does not establish
/// that anything runs. A call establishes a reference at this site; it
/// does not establish that the site is reachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferenceKind {
    Call,
    /// The function named without being called: stored, passed, returned.
    /// A real reference; where it is later invoked is a separate question.
    FunctionValue,
    /// A type named in a signature, a field, or an alias.
    Type,
    Import,
    ReExport,
}

impl ReferenceKind {
    /// Wording for a report, in a reader's terms rather than the tool's.
    pub fn describe(self) -> &'static str {
        match self {
            ReferenceKind::Call => "calls",
            ReferenceKind::FunctionValue => "uses as a value",
            ReferenceKind::Type => "names the type",
            ReferenceKind::Import => "imports",
            ReferenceKind::ReExport => "re-exports",
        }
    }
}

/// Where a reference goes, including the honest answer when that is not
/// known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Resolved to exactly one item.
    Definite(ItemId),
    /// Narrowed to a set and no further -- trait or generic dispatch, for
    /// instance. Enough to say a crossing *may* happen, never enough to
    /// say one did.
    Candidates(Vec<ItemId>),
    /// Not resolved at all, with the reason a reader needs.
    Unresolved { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub origin: ItemId,
    pub destination: Destination,
    pub kind: ReferenceKind,
    pub span: Span,
}

/// How far a gap in the analysis reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GapScope {
    /// Nothing about this module's references is known to be complete --
    /// a macro that expands to items, say.
    Module(ModuleId),
    /// One site could not be resolved; everything else about the module
    /// stands.
    Site { origin: ItemId, span: Span },
}

/// Something the analysis could not see, and what that costs.
///
/// Recorded rather than dropped: a rule whose relevant code was not read
/// is not a rule that passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageGap {
    pub scope: GapScope,
    /// Plain words, for the report: "a macro expands to items this scan
    /// cannot see".
    pub cause: String,
    /// Which kinds of reference this gap could be hiding. A gap that can
    /// only hide calls does not undermine a rule about type references.
    pub affects: Vec<ReferenceKind>,
}

/// Everything observed about one build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceModel {
    pub context: BuildContext,
    /// Every module found, including ones no component claims.
    pub modules: Vec<ModuleId>,
    pub items: Vec<ItemRecord>,
    pub references: Vec<Reference>,
    pub gaps: Vec<CoverageGap>,
}

impl SourceModel {
    pub fn new(context: BuildContext) -> Self {
        Self {
            context,
            modules: Vec::new(),
            items: Vec::new(),
            references: Vec::new(),
            gaps: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Ancestry is not a string prefix.** `parse::rhythm` starts with
    /// the text of `parse::r` and is not inside it; a containment test
    /// written with `starts_with` puts it there, and every ownership
    /// answer downstream inherits the mistake.
    #[test]
    fn containment_follows_module_boundaries_not_shared_text() {
        let short = ModuleId::parse("app::parse::r");
        let long = ModuleId::parse("app::parse::rhythm");
        assert!(!short.contains(&long), "`r` does not contain `rhythm`");

        let parse = ModuleId::parse("app::parse");
        assert!(parse.contains(&long), "but `parse` does contain it");
        assert!(parse.contains(&parse), "and contains itself");
    }

    /// Two crates in one build can share a display name, so the crate is
    /// part of the identity rather than assumed unique.
    #[test]
    fn modules_in_different_crates_never_contain_each_other() {
        let a = ModuleId::parse("one::shared");
        let b = ModuleId::parse("two::shared");
        assert!(!a.contains(&b));
        assert!(!b.contains(&a));
    }
}
