//! Source positions. One span model shared by every phase (§13).

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Index of a file inside a [`SourceMap`].
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct FileId(pub u32);

impl FileId {
    /// A file id usable before any source map exists (synthesised spans).
    pub const DUMMY: FileId = FileId(u32::MAX);
}

/// A half-open byte range `[start, end)` inside a single file.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, start: u32, end: u32) -> Span {
        debug_assert!(start <= end, "span start after end: {start}..{end}");
        Span { file, start, end }
    }

    pub const DUMMY: Span = Span { file: FileId::DUMMY, start: 0, end: 0 };

    /// Smallest span covering both. Spans from different files: `self` wins.
    pub fn to(self, other: Span) -> Span {
        if self.file != other.file {
            return self;
        }
        Span {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn len(self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// A zero-width span pinned to the end of this one; used by `insert` fixes.
    pub fn at_end(self) -> Span {
        Span { file: self.file, start: self.end, end: self.end }
    }

    /// A zero-width span pinned to the start of this one.
    pub fn at_start(self) -> Span {
        Span { file: self.file, start: self.start, end: self.start }
    }
}

/// 1-based line, 1-based column (measured in characters, not bytes).
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
struct SourceFile {
    name: String,
    text: Arc<str>,
    /// Byte offset of the first character of each line.
    line_starts: Vec<u32>,
}

fn line_starts(text: &str) -> Vec<u32> {
    let mut v = vec![0u32];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            v.push(i as u32 + 1);
        }
    }
    v
}

/// Owns every file the compiler has read; resolves spans to human coordinates.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> SourceMap {
        SourceMap::default()
    }

    pub fn add(&mut self, name: impl Into<String>, text: impl Into<Arc<str>>) -> FileId {
        let text: Arc<str> = text.into();
        let file = SourceFile { name: name.into(), line_starts: line_starts(&text), text };
        self.files.push(file);
        FileId(self.files.len() as u32 - 1)
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn ids(&self) -> impl Iterator<Item = FileId> + use<> {
        (0..self.files.len() as u32).map(FileId)
    }

    fn get(&self, file: FileId) -> Option<&SourceFile> {
        self.files.get(file.0 as usize)
    }

    pub fn name(&self, file: FileId) -> &str {
        self.get(file).map(|f| f.name.as_str()).unwrap_or("<unknown>")
    }

    pub fn text(&self, file: FileId) -> &str {
        self.get(file).map(|f| &*f.text).unwrap_or("")
    }

    pub fn source(&self, file: FileId) -> Arc<str> {
        self.get(file).map(|f| f.text.clone()).unwrap_or_else(|| Arc::from(""))
    }

    pub fn snippet(&self, span: Span) -> &str {
        let text = self.text(span.file);
        let start = (span.start as usize).min(text.len());
        let end = (span.end as usize).min(text.len());
        // Clamp to char boundaries so a truncated span never panics.
        let start = floor_boundary(text, start);
        let end = ceil_boundary(text, end.max(start));
        &text[start..end]
    }

    /// Number of lines in the file (at least 1).
    pub fn line_count(&self, file: FileId) -> u32 {
        self.get(file).map(|f| f.line_starts.len() as u32).unwrap_or(1)
    }

    /// Byte range of `line` (1-based), excluding the trailing newline.
    pub fn line_range(&self, file: FileId, line: u32) -> (u32, u32) {
        let Some(f) = self.get(file) else { return (0, 0) };
        let idx = (line.max(1) - 1) as usize;
        let Some(&start) = f.line_starts.get(idx) else {
            let end = f.text.len() as u32;
            return (end, end);
        };
        let end = f
            .line_starts
            .get(idx + 1)
            .map(|&n| n - 1)
            .unwrap_or(f.text.len() as u32);
        (start, end)
    }

    pub fn line_text(&self, file: FileId, line: u32) -> &str {
        let (s, e) = self.line_range(file, line);
        let text = self.text(file);
        &text[s as usize..e as usize]
    }

    pub fn line_col(&self, file: FileId, offset: u32) -> LineCol {
        let Some(f) = self.get(file) else { return LineCol { line: 1, col: 1 } };
        let offset = offset.min(f.text.len() as u32);
        let line_idx = match f.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let line_start = f.line_starts[line_idx] as usize;
        let col = f.text[line_start..offset as usize].chars().count() as u32 + 1;
        LineCol { line: line_idx as u32 + 1, col }
    }

    pub fn start(&self, span: Span) -> LineCol {
        self.line_col(span.file, span.start)
    }

    pub fn end(&self, span: Span) -> LineCol {
        self.line_col(span.file, span.end)
    }
}

fn floor_boundary(text: &str, mut i: usize) -> usize {
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(text: &str, mut i: usize) -> usize {
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_is_one_based() {
        let mut sm = SourceMap::new();
        let f = sm.add("a.ply", "fn main() -> () {\n    print(\"hi\");\n}\n");
        assert_eq!(sm.line_col(f, 0), LineCol { line: 1, col: 1 });
        assert_eq!(sm.line_col(f, 18), LineCol { line: 2, col: 1 });
        assert_eq!(sm.line_col(f, 22), LineCol { line: 2, col: 5 });
        assert_eq!(sm.line_text(f, 2), "    print(\"hi\");");
    }

    #[test]
    fn columns_count_characters() {
        let mut sm = SourceMap::new();
        let f = sm.add("a.ply", "let s = \"π\"; // after");
        let offset = "let s = \"π\"".len() as u32;
        assert_eq!(sm.line_col(f, offset), LineCol { line: 1, col: 12 });
    }
}
