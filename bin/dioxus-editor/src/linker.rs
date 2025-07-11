use dioxus::prelude::*;
use zosimos::command::Linker;

pub async fn from_assets() -> Result<Linker, Box<dyn std::error::Error>> {
    const STD: Asset = asset!("/assets/std.cbor");
    let std = super::asset_to_url(&STD).expect("Missing std shader assets");

    let response = reqwest::get(std).await?;
    let bytes = response.bytes().await?;

    let (core, std) = serde_cbor::from_slice(&bytes)?;
    Ok(Linker { core, std })
}
