//! Prints every audio device: `cargo run -p takkie-engine --example devices`.

use std::io::{self, Write};

use takkie_engine::audio::devices::list_devices;

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
    Ok(())
}
