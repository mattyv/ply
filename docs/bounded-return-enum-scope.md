# Module-local bounded return enums (PLY-006)

Scalar bounded contract wrappers previously imported only the crate root. A
function returning a public enum defined in its own module could compile in
production and pass sampling, while its generated Kani wrapper failed with
E0412/E0433. Re-exporting the enum at the root masked the generator defect.

Scalar wrappers and explicit byte-slice proofs now share containing-module imports.
An unqualified opaque return type receives an explicit import of its name so a
same-named root type cannot capture it. Private imported return aliases are rebound to their original
paths and names, with crate/self/super paths resolved from the containing module.

The independently authored localenum regression checks file modules, nested inline
modules, a conflicting root enum, a generic local enum, and a private standard-library
alias. A broken decision must produce K0502 and an ordinary test that fails before repair and passes
afterward. No cpp-sca source or workaround is changed. Supported output handling
does not imply support for arbitrary user-enum parameters.
