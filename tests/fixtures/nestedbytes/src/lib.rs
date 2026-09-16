pub fn valid_key_bytes(keys: &[&[u8]]) -> bool {
    if keys.is_empty() { return false; }
    let mut i = 0;
    while i < keys.len() {
        if keys[i].is_empty() { return false; }
        let mut j = 0;
        while j < i {
            if keys[i] == keys[j] { return false; }
            j += 1;
        }
        i += 1;
    }
    true
}
