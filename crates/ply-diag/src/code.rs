//! The diagnostic code registry (§13).
//!
//! Every message the compiler can emit has a code in this enum. The enum is the single
//! source of truth: it fixes the wire string, the phase, and the default severity, so the
//! JSON surface stays stable as phases are implemented.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Which pipeline stage produced a diagnostic.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Lex,
    Parse,
    Resolve,
    Type,
    Effect,
    Borrow,
    Verify,
    Run,
    Query,
    Rules,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Lex => "lex",
            Phase::Parse => "parse",
            Phase::Resolve => "resolve",
            Phase::Type => "type",
            Phase::Effect => "effect",
            Phase::Borrow => "borrow",
            Phase::Verify => "verify",
            Phase::Run => "run",
            Phase::Query => "query",
            Phase::Rules => "rules",
        }
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    Error,
    Ice,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Ice => "ice",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

macro_rules! codes {
    ($( $variant:ident = $wire:literal, $sev:ident, $phase:ident, $blurb:literal );* $(;)?) => {
        /// Every diagnostic the compiler can produce.
        #[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
        pub enum Code { $($variant),* }

        impl Code {
            pub const ALL: &'static [Code] = &[ $(Code::$variant),* ];

            /// The stable wire string, e.g. `"E0101"`.
            pub fn as_str(self) -> &'static str {
                match self { $(Code::$variant => $wire),* }
            }

            pub fn severity(self) -> Severity {
                match self { $(Code::$variant => Severity::$sev),* }
            }

            pub fn phase(self) -> Phase {
                match self { $(Code::$variant => Phase::$phase),* }
            }

            /// One-line explanation, shown by `ply explain-code` and the registry test.
            pub fn blurb(self) -> &'static str {
                match self { $(Code::$variant => $blurb),* }
            }
        }

        impl FromStr for Code {
            type Err = ();
            fn from_str(s: &str) -> Result<Code, ()> {
                match s { $($wire => Ok(Code::$variant),)* _ => Err(()) }
            }
        }
    };
}

codes! {
    // ---- E01xx: naming, lexing, parsing -------------------------------------------------
    E0101 = "E0101", Error, Resolve, "identifier violates the naming convention";
    E0102 = "E0102", Error, Lex,   "unexpected character";
    E0103 = "E0103", Error, Lex,   "unterminated string literal";
    E0104 = "E0104", Error, Lex,   "invalid escape sequence in string literal";
    E0105 = "E0105", Error, Lex,   "unterminated block comment";
    E0106 = "E0106", Error, Lex,   "integer literal is malformed";
    E0110 = "E0110", Error, Parse, "expected a specific token";
    E0111 = "E0111", Error, Parse, "unclosed delimiter";
    E0112 = "E0112", Error, Parse, "function is missing its return type";
    E0113 = "E0113", Error, Parse, "parameter is missing its type annotation";
    E0114 = "E0114", Error, Parse, "invalid assignment target";
    E0115 = "E0115", Error, Parse, "expected an expression";
    E0116 = "E0116", Error, Parse, "expected an item";
    E0117 = "E0117", Error, Parse, "expected a pattern";
    E0118 = "E0118", Error, Parse, "unknown verification mode";
    E0119 = "E0119", Error, Parse, "unknown capability";
    E0120 = "E0120", Error, Parse, "malformed query block";
    E0121 = "E0121", Error, Parse, "malformed rules block";
    E0122 = "E0122", Error, Parse, "malformed machine block";
    E0123 = "E0123", Error, Parse, "borrow annotation is only allowed on parameters";
    E0124 = "E0124", Error, Parse, "duplicate clause";

    // ---- E02xx: types --------------------------------------------------------------------
    E0201 = "E0201", Error, Type,  "type mismatch";
    E0202 = "E0202", Error, Type,  "unknown type";
    E0203 = "E0203", Error, Type,  "cannot compare values of different types";
    E0204 = "E0204", Error, Type,  "unknown field";
    E0205 = "E0205", Error, Type,  "wrong number of arguments";
    E0206 = "E0206", Error, Resolve, "unknown name";
    E0207 = "E0207", Error, Resolve, "duplicate definition";
    E0208 = "E0208", Error, Type,  "value is not callable";
    E0209 = "E0209", Error, Type,  "not indexable";
    E0210 = "E0210", Error, Type,  "generic instantiation is underdetermined";
    E0211 = "E0211", Error, Type,  "wrong number of type arguments";
    E0220 = "E0220", Error, Type,  "match is not exhaustive";
    E0221 = "E0221", Error, Type,  "unreachable match arm";
    E0222 = "E0222", Error, Type,  "assignment to an immutable binding";

    // ---- E03xx: moves and borrows --------------------------------------------------------
    E0301 = "E0301", Error, Borrow, "use of a moved value";
    E0302 = "E0302", Error, Borrow, "conflicting borrows at a call site";
    E0303 = "E0303", Error, Borrow, "cannot borrow an immutable place mutably";

    // ---- E04xx: effects ------------------------------------------------------------------
    E0401 = "E0401", Error, Effect, "missing capability";
    E0402 = "E0402", Error, Effect, "effectful expression in a pure context";

    // ---- E06xx: underspecification -------------------------------------------------------
    E0602 = "E0602", Error, Type,  "`dontcare` needs an `ensures` mentioning the result";
    E0603 = "E0603", Error, Verify, "no value can satisfy the contract at `dontcare`";

    // ---- E07xx: rules --------------------------------------------------------------------
    E0701 = "E0701", Error, Rules, "rule is not range restricted";
    E0702 = "E0702", Error, Rules, "rules are not stratified";
    E0703 = "E0703", Error, Rules, "relation arity or column type mismatch";
    E0704 = "E0704", Error, Rules, "unknown relation";

    // ---- E08xx: machines -----------------------------------------------------------------
    E0801 = "E0801", Error, Resolve, "unknown state in machine";
    E0802 = "E0802", Error, Resolve, "duplicate transition";

    // ---- V0xxx: verification -------------------------------------------------------------
    V0001 = "V0001", Error, Verify, "effectful function cannot use a solver tier";
    V0002 = "V0002", Error, Verify, "recursive call beyond depth without a contract";
    V0003 = "V0003", Error, Verify, "query in a solver-verified function";
    V0102 = "V0102", Error, Verify, "division by zero is reachable";
    V0103 = "V0103", Error, Verify, "index out of bounds is reachable";
    V0501 = "V0501", Error, Verify, "requires may fail at this call site";
    V0502 = "V0502", Error, Verify, "ensures may fail";
    V0503 = "V0503", Error, Verify, "loop invariant may fail";
    V0504 = "V0504", Error, Verify, "decreases may fail";
    V0903 = "V0903", Error, Verify, "loop has no invariant for k-induction";
    V0904 = "V0904", Error, Verify, "solver stalled";

    // ---- R0xxx: runtime traps ------------------------------------------------------------
    R0101 = "R0101", Error, Run,   "explicit trap";
    R0102 = "R0102", Error, Run,   "division by zero";
    R0103 = "R0103", Error, Run,   "list index out of bounds";
    R0104 = "R0104", Error, Run,   "call depth limit exceeded";
    R0501 = "R0501", Error, Run,   "requires failed";
    R0502 = "R0502", Error, Run,   "ensures failed";
    R0503 = "R0503", Error, Run,   "loop invariant failed";
    R0601 = "R0601", Error, Run,   "unresolved decision reached";
    R0801 = "R0801", Error, Run,   "machine invariant failed";

    // ---- W0xxx: warnings -----------------------------------------------------------------
    W0601 = "W0601", Warning, Parse, "unresolved decision recorded on the worklist";
    W0802 = "W0802", Warning, Resolve, "machine state is unreachable or has no path to a terminal";
    W0902 = "W0902", Warning, Verify, "requires is too tight to fuzz";

    // ---- X0xxx: internal self-checks -----------------------------------------------------
    X0001 = "X0001", Ice, Verify, "counterexample did not reproduce on the VM";
    X0002 = "X0002", Ice, Verify, "prove and bounded disagree";
    X0003 = "X0003", Ice, Parse, "internal compiler error";
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn wire_strings_are_unique_and_roundtrip() {
        let mut seen = HashSet::new();
        for &c in Code::ALL {
            assert!(seen.insert(c.as_str()), "duplicate code {}", c.as_str());
            assert_eq!(Code::from_str(c.as_str()), Ok(c));
        }
    }

    #[test]
    fn severity_matches_the_code_range() {
        for &c in Code::ALL {
            let expected = match c.as_str().as_bytes()[0] {
                b'W' => Severity::Warning,
                b'X' => Severity::Ice,
                _ => Severity::Error,
            };
            assert_eq!(c.severity(), expected, "{c} has the wrong default severity");
        }
    }
}
