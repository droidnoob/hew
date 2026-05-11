use std::error::Error;

use vergen::Emitter;
use vergen::{BuildBuilder, CargoBuilder, RustcBuilder};

fn main() -> Result<(), Box<dyn Error>> {
    Emitter::default()
        .add_instructions(&BuildBuilder::all_build()?)?
        .add_instructions(&CargoBuilder::all_cargo()?)?
        .add_instructions(&RustcBuilder::all_rustc()?)?
        .emit()?;
    Ok(())
}
