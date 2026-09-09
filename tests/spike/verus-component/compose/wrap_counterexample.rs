// Does a program that SATISFIES the contract still break the invariant?
// The contract, exactly as entail.rs models it: on success, available
// becomes old(available) - tokens, and capacity is unchanged.
fn main() {
    let capacity: u32 = 5;
    let available: u32 = 3;
    let tokens: u32 = 4;

    // An implementation that satisfies "available == old(available) - tokens"
    // under Rust's own arithmetic.
    let new_available = available.wrapping_sub(tokens);

    // Is the promise, as a Rust expression, true?
    let promise_holds = new_available == available.wrapping_sub(tokens);
    // Is the invariant true?
    let invariant_holds = new_available <= capacity;

    println!("available {available} -> {new_available}, capacity {capacity}");
    println!("promise holds:   {promise_holds}");
    println!("invariant holds: {invariant_holds}");
}
