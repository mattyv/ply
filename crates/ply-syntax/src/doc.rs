//! A small Wadler-style document combinator engine, used by `ply fmt`.
//!
//! The formatter builds a `Doc` and the renderer decides where to break: a `Group` is laid
//! out flat if it fits in the remaining width, otherwise every `Line` inside it becomes a
//! newline. That is what makes the output canonical — the layout is a function of the tree
//! and the width alone, with no formatter options and no dependence on the input layout.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Doc {
    Nil,
    Text(String),
    /// A space when flat, a newline when broken.
    Line,
    /// Nothing when flat, a newline when broken.
    SoftLine,
    /// Always a newline; forces every enclosing group to break.
    HardLine,
    /// `(flat, broken)` — e.g. a trailing comma that only appears when broken.
    IfBreak(String, String),
    Concat(Vec<Doc>, bool),
    Nest(usize, Box<Doc>, bool),
    Group(Box<Doc>, bool),
}

impl Doc {
    /// True when the document contains a hard break, so enclosing groups must break too.
    pub fn is_hard(&self) -> bool {
        match self {
            Doc::HardLine => true,
            Doc::Concat(_, h) | Doc::Nest(_, _, h) | Doc::Group(_, h) => *h,
            _ => false,
        }
    }
}

pub fn nil() -> Doc {
    Doc::Nil
}

pub fn text(s: impl Into<String>) -> Doc {
    Doc::Text(s.into())
}

pub fn line() -> Doc {
    Doc::Line
}

pub fn softline() -> Doc {
    Doc::SoftLine
}

pub fn hardline() -> Doc {
    Doc::HardLine
}

pub fn if_break(broken: impl Into<String>, flat: impl Into<String>) -> Doc {
    Doc::IfBreak(flat.into(), broken.into())
}

pub fn concat(parts: Vec<Doc>) -> Doc {
    let hard = parts.iter().any(Doc::is_hard);
    Doc::Concat(parts, hard)
}

pub fn nest(n: usize, d: Doc) -> Doc {
    let hard = d.is_hard();
    Doc::Nest(n, Box::new(d), hard)
}

pub fn group(d: Doc) -> Doc {
    let hard = d.is_hard();
    Doc::Group(Box::new(d), hard)
}

/// `parts` interleaved with `sep`.
pub fn join(sep: Doc, parts: Vec<Doc>) -> Doc {
    let mut out = Vec::with_capacity(parts.len() * 2);
    for (i, p) in parts.into_iter().enumerate() {
        if i > 0 {
            out.push(sep.clone());
        }
        out.push(p);
    }
    concat(out)
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Mode {
    Flat,
    Break,
}

pub fn render(root: &Doc, width: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    let mut stack: Vec<(usize, Mode, &Doc)> = vec![(0, Mode::Break, root)];
    while let Some((ind, mode, d)) = stack.pop() {
        match d {
            Doc::Nil => {}
            Doc::Text(s) => {
                out.push_str(s);
                // Block comments are emitted verbatim and may span lines.
                match s.rfind('\n') {
                    Some(i) => col = s[i + 1..].chars().count(),
                    None => col += s.chars().count(),
                }
            }
            Doc::IfBreak(flat, broken) => {
                let s = if mode == Mode::Flat { flat } else { broken };
                out.push_str(s);
                col += s.chars().count();
            }
            Doc::Line if mode == Mode::Flat => {
                out.push(' ');
                col += 1;
            }
            Doc::SoftLine if mode == Mode::Flat => {}
            Doc::Line | Doc::SoftLine | Doc::HardLine => {
                // Never leave trailing whitespace on a line.
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push('\n');
                out.push_str(&" ".repeat(ind));
                col = ind;
            }
            Doc::Concat(parts, _) => {
                for p in parts.iter().rev() {
                    stack.push((ind, mode, p));
                }
            }
            Doc::Nest(n, inner, _) => stack.push((ind + n, mode, inner)),
            Doc::Group(inner, hard) => {
                let m = if *hard || !fits(width as isize - col as isize, ind, inner, &stack) {
                    Mode::Break
                } else {
                    Mode::Flat
                };
                stack.push((ind, m, inner));
            }
        }
    }
    out
}

/// Would `group` plus whatever follows on this line fit in `rem` columns?
fn fits(mut rem: isize, ind: usize, group: &Doc, rest: &[(usize, Mode, &Doc)]) -> bool {
    let mut local: Vec<(usize, Mode, &Doc)> = vec![(ind, Mode::Flat, group)];
    let mut rest_i = rest.len();
    loop {
        if rem < 0 {
            return false;
        }
        let (i, mode, d) = match local.pop() {
            Some(item) => item,
            None => {
                if rest_i == 0 {
                    return true;
                }
                rest_i -= 1;
                rest[rest_i]
            }
        };
        match d {
            Doc::Nil => {}
            Doc::Text(s) if s.contains('\n') => return false,
            Doc::Text(s) => rem -= s.chars().count() as isize,
            Doc::IfBreak(flat, broken) => {
                let s = if mode == Mode::Flat { flat } else { broken };
                rem -= s.chars().count() as isize;
            }
            Doc::Line if mode == Mode::Flat => rem -= 1,
            Doc::SoftLine if mode == Mode::Flat => {}
            Doc::HardLine if mode == Mode::Flat => return false,
            Doc::Line | Doc::SoftLine | Doc::HardLine => return true,
            Doc::Concat(parts, _) => {
                for p in parts.iter().rev() {
                    local.push((i, mode, p));
                }
            }
            Doc::Nest(n, inner, _) => local.push((i + n, mode, inner)),
            // A nested group is measured flat, unless it contains a hard break.
            Doc::Group(inner, hard) => {
                local.push((i, if *hard { Mode::Break } else { Mode::Flat }, inner))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(n: usize) -> Doc {
        let items: Vec<Doc> = (0..n).map(|i| text(format!("arg{i}"))).collect();
        group(concat(vec![
            text("f("),
            nest(4, concat(vec![softline(), join(concat(vec![text(","), line()]), items), if_break(",", "")])),
            softline(),
            text(")"),
        ]))
    }

    #[test]
    fn groups_stay_flat_when_they_fit() {
        assert_eq!(render(&args(2), 40), "f(arg0, arg1)");
    }

    #[test]
    fn groups_break_one_item_per_line() {
        assert_eq!(render(&args(3), 12), "f(\n    arg0,\n    arg1,\n    arg2,\n)");
    }

    #[test]
    fn hard_breaks_propagate_to_enclosing_groups() {
        let d = group(concat(vec![text("a"), line(), hardline(), text("b")]));
        assert_eq!(render(&d, 100), "a\n\nb");
        assert!(d.is_hard());
    }

    #[test]
    fn nesting_indents_continuation_lines() {
        let d = group(concat(vec![text("x ="), nest(4, concat(vec![line(), text("value")]))]));
        assert_eq!(render(&d, 5), "x =\n    value");
        assert_eq!(render(&d, 40), "x = value");
    }

    #[test]
    fn breaks_never_leave_trailing_spaces() {
        let d = concat(vec![text("a"), text(" "), hardline(), text("b")]);
        assert_eq!(render(&d, 80), "a\nb");
    }

    #[test]
    fn what_follows_the_group_counts_towards_the_width() {
        // The group itself fits in 10 columns, but the `;;;;;` after it does not.
        let d = concat(vec![args(1), text(";;;;;;;;;;")]);
        assert_eq!(render(&d, 14), "f(\n    arg0,\n);;;;;;;;;;");
    }
}
