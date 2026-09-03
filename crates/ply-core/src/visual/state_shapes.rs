//! The shape a state field is drawn as (§7.1).
//!
//! `state:` names a type and the fields of it worth seeing. What those
//! fields *are* is a fact about code, not about the document -- so this
//! module takes the type the source scanner actually found and picks one of
//! seven silhouettes for it.
//!
//! **Ink only, no new colour.** Every hue in a Ply drawing is already
//! spoken for: green is evidence a run earned, red is a violation, violet
//! is authorship, and the grey ramp is the declared ceiling. A shape
//! channel can be added without disturbing any of them; a colour channel
//! could not. The seven forms are drawn to be distinct silhouettes at 12px,
//! which is the size they are actually read at -- a first draft of the
//! proposal sheet had two that were not, and was thrown away after
//! rasterising it (`docs/state-shapes.svg` is the survivor).
//!
//! The eighth case is deliberately not an eighth form: a field Ply has no
//! way to build a value of carries the same diagonal hatching unclaimed
//! code already carries, on the glyph itself. That is usually the single
//! most useful thing a reader can be shown about a component, because it is
//! the reason its functions come back unsupported.

use crate::harness::RustType;

/// One of §7.1's seven state-field silhouettes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldShape {
    /// A filled cell. One value, no interior.
    Scalar,
    /// A cell with a written line across it.
    Text,
    /// Three stacked equal bars -- order is carried.
    List,
    /// A narrow key cell beside a wide value cell, twice.
    Map,
    /// Three loose discs, unaligned -- each once, no order.
    Set,
    /// A dashed cell. `Option`: the value might not be there.
    Maybe,
    /// Two overlapping outlined cells -- a struct or enum of your own,
    /// which is to say there is more inside than this row shows.
    Own,
}

impl FieldShape {
    /// The name this shape is called by in prose, at the newbie bar: what a
    /// reader would say they were looking at, never a Rust type name.
    pub fn noun(self) -> &'static str {
        match self {
            FieldShape::Scalar => "a single value",
            FieldShape::Text => "text",
            FieldShape::List => "a list",
            FieldShape::Map => "a lookup table",
            FieldShape::Set => "a set",
            FieldShape::Maybe => "something that might not be there",
            FieldShape::Own => "a structure of your own",
        }
    }

    /// The sentence a reader gets on hover. Names the visual first (the
    /// glyphs are unusual and nobody is born knowing them), then what it
    /// means -- the order §7.1's newbie-bar rule asks for.
    pub fn prose(self) -> &'static str {
        match self {
            FieldShape::Scalar => {
                "the filled block means one single value -- a number, a flag, a character"
            }
            FieldShape::Text => "the block with a line written across it means text",
            FieldShape::List => {
                "the three stacked bars mean a list: many values, kept in the order they \
                 were put in"
            }
            FieldShape::Map => {
                "the narrow cell beside a wide one, twice, means a lookup table: you hand \
                 it a key and it gives you back a value"
            }
            FieldShape::Set => {
                "the three loose circles mean a set: each value appears once, and there is \
                 no order to them"
            }
            FieldShape::Maybe => "the dashed outline means the value might not be there at all",
            FieldShape::Own => {
                "the two overlapping outlines mean a structure of your own -- there is more \
                 inside it than this row shows"
            }
        }
    }
}

/// Picks a shape for one field.
///
/// Two sources, in that order. [`RustType`] is consulted first because it
/// is a parsed fact; the rendered source text is the fallback for
/// everything the harness's own vocabulary calls
/// [`RustType::Unsupported`] -- which is most real state, since that
/// vocabulary exists to answer "can Ply *build* one of these" and a state
/// field only has to be *described*. A `HashMap` Ply cannot generate is
/// still, visibly, a lookup table, and drawing it as an anonymous blob
/// would throw away something the reader can plainly see in the source.
///
/// A wrapper that says something about presence wins over what it wraps:
/// `Option<Vec<T>>` draws as "might not be there", because that is the
/// fact a reader needs first. A wrapper that says nothing (`Box`, `Arc`,
/// `Rc`, `RefCell`) is transparent and the shape inside it shows through.
pub fn classify(ty: &RustType, rendered: &str) -> FieldShape {
    match ty {
        RustType::Option(_) => FieldShape::Maybe,
        RustType::String => FieldShape::Text,
        RustType::Vec(_) | RustType::VecU8 | RustType::Array(_, _) | RustType::Slice(_) => {
            FieldShape::List
        }
        RustType::BTreeSet(_) => FieldShape::Set,
        RustType::BTreeMap(_, _) => FieldShape::Map,
        RustType::BoxT(inner) => classify(inner, strip_wrapper(rendered)),
        RustType::U8
        | RustType::U16
        | RustType::U32
        | RustType::U64
        | RustType::I8
        | RustType::I16
        | RustType::I32
        | RustType::I64
        | RustType::Usize
        | RustType::Isize
        | RustType::Bool
        | RustType::Char
        | RustType::F32
        | RustType::F64
        | RustType::NonZero(_)
        | RustType::Duration
        | RustType::Unit => FieldShape::Scalar,
        RustType::Tuple(_)
        | RustType::Result(_, _)
        | RustType::SelfType
        | RustType::UserTypeCtor(_)
        | RustType::UserTypeFields(_) => FieldShape::Own,
        RustType::Unsupported(_) => from_source_text(rendered),
    }
}

/// The shape of a type the harness has no vocabulary for, read off the
/// source text. Matches on the outermost named constructor only -- the
/// generic arguments describe what is *inside* the shape, not the shape.
fn from_source_text(rendered: &str) -> FieldShape {
    let head = head_of(rendered);
    // Transparent wrappers: they change ownership or mutability, never what
    // the thing is. A `Arc<RwLock<Vec<T>>>` is a list to anyone reading the
    // picture, and drawing it as an anonymous blob would hide that.
    if matches!(
        head,
        "Box" | "Arc" | "Rc" | "RefCell" | "Cell" | "Mutex" | "RwLock" | "UnsafeCell"
    ) {
        let inner = strip_wrapper(rendered);
        if inner != rendered {
            return from_source_text(inner);
        }
    }
    match head {
        "Option" => FieldShape::Maybe,
        "String" | "str" | "OsString" | "PathBuf" | "Cow" => FieldShape::Text,
        "Vec" | "VecDeque" | "SmallVec" | "ArrayVec" | "BinaryHeap" => FieldShape::List,
        "HashMap" | "BTreeMap" | "IndexMap" | "DashMap" | "HashBiMap" => FieldShape::Map,
        "HashSet" | "BTreeSet" | "IndexSet" => FieldShape::Set,
        // The primitives, so this path answers the same as the parsed one
        // for anything the harness could also have named. They reach here
        // through a type alias, a `cfg`-guarded width, or any other spelling
        // the parser declined -- and a number drawn as an anonymous
        // structure would be a plain misreading of the source.
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
        | "isize" | "f32" | "f64" | "bool" | "char" | "NonZeroU8" | "NonZeroU16" | "NonZeroU32"
        | "NonZeroU64" | "NonZeroUsize" | "NonZeroI8" | "NonZeroI16" | "NonZeroI32"
        | "NonZeroI64" | "NonZeroIsize" | "Duration" | "Instant" | "AtomicU8" | "AtomicU16"
        | "AtomicU32" | "AtomicU64" | "AtomicUsize" | "AtomicBool" | "AtomicI8" | "AtomicI16"
        | "AtomicI32" | "AtomicI64" | "AtomicIsize" => FieldShape::Scalar,
        _ => {
            // `[T; N]` and `&[T]` have no head identifier to match on.
            let t = rendered.trim_start_matches('&').trim();
            if t.starts_with('[') {
                FieldShape::List
            } else {
                FieldShape::Own
            }
        }
    }
}

/// The outermost type constructor's name: `BTreeMap` of
/// `BTreeMap<u64, Level>`, `Vec` of `Vec<Slot<T>>`. Empty when the text
/// starts with something that is not an identifier.
fn head_of(rendered: &str) -> &str {
    let t = rendered
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim();
    let t = t.rsplit("::").next().unwrap_or(t);
    let end = t
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(t.len());
    &t[..end]
}

/// The first generic argument of a one-argument wrapper, or the whole text
/// unchanged when there is no such argument to take.
fn strip_wrapper(rendered: &str) -> &str {
    let Some(open) = rendered.find('<') else {
        return rendered;
    };
    let Some(close) = rendered.rfind('>') else {
        return rendered;
    };
    if close <= open + 1 {
        return rendered;
    }
    rendered[open + 1..close].trim()
}

/// Whether Ply has no way to make a value of this field's type, which is
/// drawn as the hatch the unclaimed ceiling already uses.
///
/// Deliberately keyed on the harness's own verdict rather than on the
/// shape: a lookup table draws as a lookup table *and* hatched, because
/// both facts are true and they answer different questions. The shape says
/// what the field is; the hatch says why the functions around it may come
/// back unsupported.
///
/// The predicate is the sampling engine's own [`RustType::is_fuzz_supported`]
/// rather than "the parser gave up on it". Measured, and the difference is
/// not small: `BTreeMap<u64, Level>` parses perfectly well as a map and
/// still cannot be built, because nothing can build a `Level`. Asking
/// whether the *parse* failed missed all three unbuildable fields in a
/// fixture written to have three.
pub fn cannot_build(ty: &RustType) -> bool {
    !ty.is_fuzz_supported()
}

/// The glyph, drawn at `(x, y)` as its top-left corner inside a
/// [`GLYPH_W`]x[`GLYPH_H`] cell.
///
/// Geometry taken from the reviewed proposal sheet (`docs/state-shapes.svg`)
/// rather than reinvented, so what shipped is what was looked at.
pub fn glyph_svg(shape: FieldShape, x: f64, y: f64, hatched: bool) -> String {
    // The hatch replaces the glyph's own fill, so a hatched glyph keeps its
    // silhouette -- a reader still sees *which* shape cannot be built,
    // rather than a hatched rectangle that could be any of the seven.
    let ink = if hatched {
        "state-glyph-ink state-glyph-hatched"
    } else {
        "state-glyph-ink"
    };
    let out = "state-glyph-out";
    match shape {
        FieldShape::Scalar => format!(
            "<rect class=\"{ink}\" x=\"{:.1}\" y=\"{:.1}\" width=\"10\" height=\"10\" rx=\"1.5\" />",
            x + 2.0,
            y + 1.5
        ),
        FieldShape::Text => format!(
            "<rect class=\"{out}\" x=\"{:.1}\" y=\"{:.1}\" width=\"15\" height=\"10\" rx=\"1.5\" />\
             <rect class=\"{ink}\" x=\"{:.1}\" y=\"{:.1}\" width=\"10\" height=\"1.8\" />",
            x,
            y + 1.5,
            x + 2.5,
            y + 7.9
        ),
        FieldShape::List => format!(
            "<rect class=\"{ink}\" x=\"{x:.1}\" y=\"{:.1}\" width=\"15\" height=\"2.6\" />\
             <rect class=\"{ink}\" x=\"{x:.1}\" y=\"{:.1}\" width=\"15\" height=\"2.6\" />\
             <rect class=\"{ink}\" x=\"{x:.1}\" y=\"{:.1}\" width=\"15\" height=\"2.6\" />",
            y + 1.0,
            y + 5.4,
            y + 9.8
        ),
        FieldShape::Map => format!(
            "<rect class=\"{ink}\" x=\"{x:.1}\" y=\"{:.1}\" width=\"5\" height=\"4.6\" />\
             <rect class=\"{out}\" x=\"{:.1}\" y=\"{:.1}\" width=\"12\" height=\"4.6\" />\
             <rect class=\"{ink}\" x=\"{x:.1}\" y=\"{:.1}\" width=\"5\" height=\"4.6\" />\
             <rect class=\"{out}\" x=\"{:.1}\" y=\"{:.1}\" width=\"12\" height=\"4.6\" />",
            y + 1.2,
            x + 7.0,
            y + 1.2,
            y + 8.0,
            x + 7.0,
            y + 8.0
        ),
        FieldShape::Set => format!(
            "<rect class=\"{ink}\" x=\"{x:.1}\" y=\"{:.1}\" width=\"6\" height=\"6\" rx=\"3\" />\
             <rect class=\"{ink}\" x=\"{:.1}\" y=\"{:.1}\" width=\"6\" height=\"6\" rx=\"3\" />\
             <rect class=\"{ink}\" x=\"{:.1}\" y=\"{:.1}\" width=\"6\" height=\"6\" rx=\"3\" />",
            y,
            x + 9.0,
            y + 4.5,
            x + 3.5,
            y + 7.0
        ),
        // The two outline-only forms have no ink to swap for the hatch, so
        // the hatch becomes their *fill*. Without this the hatch could not
        // reach them at all -- and "a structure of your own" and "something
        // Ply has no vocabulary for" are the two commonest unbuildable
        // fields there are, which would have left the hatch unable to say
        // the thing it exists to say. Found by a test, not by review: a
        // fixture written with three unbuildable fields drew one hatch.
        FieldShape::Maybe => format!(
            "<rect class=\"{out} state-glyph-dashed{fill}\" x=\"{x:.1}\" y=\"{:.1}\" \
             width=\"12\" height=\"10\" rx=\"1.5\" />",
            y + 1.5,
            fill = if hatched { " state-glyph-hatched" } else { "" },
        ),
        FieldShape::Own => format!(
            "<rect class=\"{out}\" x=\"{:.1}\" y=\"{y:.1}\" width=\"12\" height=\"9\" rx=\"1.5\" />\
             <rect class=\"{out} {front}\" x=\"{x:.1}\" y=\"{:.1}\" width=\"12\" \
             height=\"9\" rx=\"1.5\" />",
            x + 4.0,
            y + 4.0,
            // The front cell is normally painted in the page's own
            // background so the cell behind it reads as *behind*; hatched,
            // the hatch does that job and says the extra thing.
            front = if hatched {
                "state-glyph-hatched"
            } else {
                "state-glyph-front"
            },
        ),
    }
}

/// Cell the glyph is drawn inside. Wide enough for the widest form (the
/// map's key-beside-value pair, 19 units) with a little air after it.
pub const GLYPH_W: f64 = 19.0;
/// Tall enough for the tallest form (the set's third disc, 13 units).
pub const GLYPH_H: f64 = 13.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// The classifier's whole job is to name what a reader can see in the
    /// source. These are the shapes real state is made of, written as the
    /// source writes them.
    #[test]
    fn a_field_is_drawn_as_what_the_source_says_it_is() {
        let cases: &[(&str, FieldShape)] = &[
            ("u64", FieldShape::Scalar),
            ("bool", FieldShape::Scalar),
            ("String", FieldShape::Text),
            ("Cow<'a, str>", FieldShape::Text),
            ("Vec<Slot<T>>", FieldShape::List),
            ("VecDeque<Tick>", FieldShape::List),
            ("[u32; 4]", FieldShape::List),
            ("HashMap<Price, Level>", FieldShape::Map),
            ("BTreeMap<u64, Level>", FieldShape::Map),
            ("std::collections::HashSet<OrderId>", FieldShape::Set),
            ("Option<Tick>", FieldShape::Maybe),
            ("OrderBook", FieldShape::Own),
            ("Arc<RwLock<Vec<Tick>>>", FieldShape::List),
        ];
        for (source, want) in cases {
            let got = classify(&RustType::Unsupported((*source).into()), source);
            assert_eq!(
                got,
                *want,
                "`{source}` should be drawn as {}, not {}",
                want.noun(),
                got.noun()
            );
        }
    }

    /// A parsed type beats the source text, because it is the stronger
    /// fact -- and the two must not disagree about anything real.
    #[test]
    fn a_parsed_type_is_trusted_over_its_source_text() {
        assert_eq!(
            classify(&RustType::Option(Box::new(RustType::U64)), "Option<u64>"),
            FieldShape::Maybe
        );
        assert_eq!(
            classify(&RustType::Vec(Box::new(RustType::U32)), "Vec<u32>"),
            FieldShape::List
        );
        assert_eq!(classify(&RustType::String, "String"), FieldShape::Text);
    }

    /// Whether Ply can build a value is a separate question from what the
    /// field is, and the drawing answers both at once. A lookup table Ply
    /// cannot generate is still drawn as a lookup table.
    #[test]
    fn a_shape_ply_cannot_build_keeps_its_own_silhouette() {
        let ty = RustType::Unsupported("HashMap<Price, Level>".into());
        assert_eq!(
            classify(&ty, "HashMap<Price, Level>"),
            FieldShape::Map,
            "a map Ply has no strategy for is still visibly a map"
        );
        assert!(cannot_build(&ty), "nothing here can build a `Level`");
        assert!(!cannot_build(&RustType::U64), "a number is buildable");
        assert!(
            cannot_build(&RustType::BTreeMap(
                Box::new(RustType::U64),
                Box::new(RustType::Unsupported("Level".into())),
            )),
            "a map parses as a map and is still unbuildable when its values are -- the \
             hatch has to follow what can be built, not what could be parsed"
        );
    }

    /// Every glyph must actually paint something. A shape that emits an
    /// empty string draws a row with a blank where its meaning goes, and
    /// no structural test would notice.
    #[test]
    fn every_shape_paints_something() {
        for shape in [
            FieldShape::Scalar,
            FieldShape::Text,
            FieldShape::List,
            FieldShape::Map,
            FieldShape::Set,
            FieldShape::Maybe,
            FieldShape::Own,
        ] {
            let svg = glyph_svg(shape, 0.0, 0.0, false);
            assert!(svg.contains("<rect"), "{} paints nothing", shape.noun());
            assert!(
                !svg.contains("{ink}") && !svg.contains("{out}"),
                "{} left an unsubstituted class placeholder: {svg}",
                shape.noun()
            );
            // Every form, without exception. The two outline-only ones are
            // the point: a structure of your own and a shape Ply has no
            // vocabulary for are the commonest unbuildable fields, so a
            // hatch that could not reach them would be unable to say the
            // one thing it is for.
            let hatched = glyph_svg(shape, 0.0, 0.0, true);
            assert!(
                hatched.contains("state-glyph-hatched"),
                "{} cannot be drawn as something Ply has no way to build",
                shape.noun()
            );
        }
    }

    /// Two glyphs that render identically are one glyph with two meanings,
    /// which is the defect the first draft of the proposal sheet had.
    #[test]
    fn no_two_shapes_draw_the_same_thing() {
        let all = [
            FieldShape::Scalar,
            FieldShape::Text,
            FieldShape::List,
            FieldShape::Map,
            FieldShape::Set,
            FieldShape::Maybe,
            FieldShape::Own,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(
                    glyph_svg(*a, 0.0, 0.0, false),
                    glyph_svg(*b, 0.0, 0.0, false),
                    "{} and {} draw the same glyph",
                    a.noun(),
                    b.noun()
                );
            }
        }
    }
}
