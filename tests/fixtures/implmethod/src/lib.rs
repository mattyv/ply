//! Method resolution (The-Ply-Spec.md §5.2, §5.1a rule 3: `Type::method`).
//!
//! Every shape this fixture exercises is real, ordinary Rust, never bent
//! toward Ply: a constructor with no receiver, a method with one, two impl
//! blocks for the same type that do not collide, a free function that
//! happens to share a name with a method, a trait method, and a generic
//! impl block.

/// A struct with one non-generic, inherent `impl` block: the plain case.
pub struct Bucket {
    capacity: u32,
}

impl Bucket {
    /// Receiverless associated function -- a constructor. There is no
    /// receiver to build, so nothing but resolution ever blocked this: once
    /// `Bucket::new` resolves, its `u32` parameter and struct return type
    /// are both already inside Ply's supported set, and it is fully
    /// checkable.
    #[ply::requires(cap > 0)]
    #[ply::ensures(|result| result.capacity == cap)]
    pub fn new(cap: u32) -> Self {
        Bucket { capacity: cap }
    }

    /// A method with a `&self` receiver -- and, since `Bucket` has its own
    /// constructor above, exactly the shape `discover_method_with_receiver`
    /// now builds a receiver for (docs/review-self-construction.md's
    /// "fourth option", 2026-08-27): Ply calls `Bucket::new` itself, runs a
    /// bounded sequence of `Bucket`'s own operations against the result (an
    /// empty pool here -- there is no other same-shape sibling operation on
    /// this type, so the pool is `capacity` repeated), then calls
    /// `capacity` and checks its postcondition. No struct literal, no
    /// declared invariant.
    #[ply::ensures(|result| *result == *result)]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Receiverless, `u32` in and out -- the exact shape a broken harness
    /// was found against (adversarial review, 2026-08-27): the fuzz-tier
    /// harness crate tried to `use crate::Bucket::clamped;`, which does not
    /// compile (a method is not an importable item), so this claim came
    /// back `tool_error`/`X0901` ("failed to compile") instead of a real
    /// `fuzzed(32)` verdict.
    #[ply::requires(cap <= 1_000)]
    #[ply::ensures(|result| *result <= 1_000)]
    pub fn clamped(cap: u32) -> u32 {
        cap.min(1_000)
    }
}

/// A plain struct, unrelated to `Bucket` -- stands in for "some type Ply's
/// parser does not model", in return position.
pub struct Elsewhere {
    pub n: u32,
}

impl Bucket {
    /// Receiverless, and its *return* type (not `Self`, not a parameter) is
    /// a struct Ply's parser does not recognise. Must be refused honestly
    /// as unsupported -- never a broken harness -- exactly the way an
    /// unrecognised *parameter* type already is.
    #[ply::requires(true)]
    #[ply::ensures(|result| result.n == 0)]
    pub fn make_elsewhere() -> Elsewhere {
        Elsewhere { n: 0 }
    }
}

/// A free function sharing a name with `Bucket::capacity`. The two must
/// never resolve to each other: a bare `capacity` claim means this
/// function, and only a `Bucket::capacity` claim means the method.
#[ply::ensures(|result| *result == 7)]
pub fn capacity() -> u32 {
    7
}

/// Two impl blocks for one type, each defining a different method -- legal
/// Rust, and both must resolve independently (no ambiguity: no method name
/// is defined twice).
pub struct Meter;

impl Meter {
    /// Receiverless, zero parameters, contracted, checked on the *fuzz*
    /// tier. Found broken (adversarial review, 2026-08-27) independently of
    /// the method-import bug above: with no parameters to generate,
    /// `fuzz_gen`'s combined strategy expression was a bare `()` -- a
    /// value, not a `proptest::strategy::Strategy` -- so *every*
    /// zero-parameter fuzz claim failed to compile ("the trait bound `()`:
    /// Strategy is not satisfied"), regardless of whether it was a method
    /// or a free function. `FakeClock::new()` in the rate-limiter fixture
    /// is this exact shape.
    #[ply::ensures(|result| true)]
    pub fn zero() -> Self {
        Meter
    }
}

impl Meter {
    #[ply::ensures(|result| *result == 100)]
    pub fn centimeters_per_meter() -> u32 {
        100
    }
}

/// A trait with a method, plus a type that implements it. `Widget::size`
/// (declared on the trait) and `Gadget::size` (the trait's implementation
/// for `Gadget`) are both real, resolvable methods -- and both out of
/// scope: Ply checks inherent methods and free functions, not trait
/// methods, yet.
pub trait Widget {
    fn size(&self) -> u32;
}

pub struct Gadget;

impl Widget for Gadget {
    fn size(&self) -> u32 {
        1
    }
}

/// A generic inherent `impl` block. `Pair::describe` resolves -- Ply finds
/// it -- but is refused: generic `impl` blocks are out of scope this task.
pub struct Pair<T> {
    pub value: T,
}

impl<T> Pair<T> {
    pub fn describe(&self) -> u32 {
        0
    }
}
