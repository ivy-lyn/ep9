//! The shipped binary of an fp-check fixture: it calls the case's engine code with inputs the
//! optimizer cannot see.
use std::hint::black_box;

fn main() {
    let x = black_box(std::env::args().count() as f64 + 0.5);
    println!("{}", fx_engine::run(x));
}
