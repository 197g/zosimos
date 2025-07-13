use std::{env, fs, path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let linker = zosimos_std::from_included();

    let target_dir = env::var_os("OUT_DIR").unwrap();
    let target_dir = path::Path::new(&target_dir).join("zosimos");

    fs::create_dir_all(&target_dir)?;
    let std = target_dir.join("std.cbor");

    let file = fs::OpenOptions::new().write(true).create(true).open(&std)?;

    serde_cbor::to_writer(file, &(&linker.core, &linker.std))?;

    let _ = fs::hard_link(std, concat!(env!("CARGO_MANIFEST_DIR"), "/assets/std.cbor"));

    Ok(())
}
