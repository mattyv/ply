use crate::types::Window;

impl Window {
    /// Written in Ply's own notation, in a different file from `Window`
    /// itself -- the constructor scan must find this precondition, not only
    /// the constructor.
    #[ply::requires(start <= end)]
    pub fn new(start: u32, end: u32) -> Self {
        Window { start, end }
    }
}
