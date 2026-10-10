//! Prints every audio device, then which ones a name would pick:
//! `cargo run -p takkie-engine --example devices -- [input name] [output name]`.

use std::io::{self, Write};

use takkie_engine::audio::devices::{Direction, find_device, list_devices};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let list = list_devices()?;
    let mut out = io::stdout().lock();
    writeln!(out, "Inputs:")?;
    for device in &list.inputs {
        writeln!(out, "  {device}")?;
    }
    writeln!(out, "Outputs:")?;
    for device in &list.outputs {
        writeln!(out, "  {device}")?;
    }

    let mut args = std::env::args().skip(1);
    for direction in [Direction::Input, Direction::Output] {
        let wanted = args.next();
        let chosen = find_device(direction, wanted.as_deref())?;
        writeln!(out, "Picked {direction}: {}", chosen.name)?;
        if let Some(warning) = chosen.warning {
            writeln!(out, "  warning: {warning}")?;
        }
    }
    Ok(())
}
