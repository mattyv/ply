use crate::widgets::Widget;

impl Widget {
    /// A correct claim in a multi-module crate: `Widget` is declared in
    /// `widgets.rs`, its `impl` block is here in a completely different
    /// file -- real, ordinary Rust -- and this must still resolve and
    /// check normally, exactly as it would if both sat in one file.
    #[ply::ensures(|result| *result == 3)]
    pub fn three() -> u32 {
        3
    }

    /// Claimed only through the re-exported name `ExportedWidget::four`
    /// (see `lib.rs`'s `pub use widgets::Widget as ExportedWidget;`) -- a
    /// type re-exported under another name must still resolve its methods
    /// to the exact same declaration a claim spelled with its real name
    /// would reach.
    #[ply::ensures(|result| *result == 4)]
    pub fn four() -> u32 {
        4
    }
}
