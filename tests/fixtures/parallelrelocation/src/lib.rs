#[ply::ensures(|result| *result == x)]
pub fn ordinary(x: bool) -> bool {
    x
}

#[ply::ensures(|result| !*result)]
pub fn path_sensitive(x: bool) -> bool {
    env!("CARGO_MANIFEST_DIR").as_bytes()[5] == b'.' && x
}
