use dioxus::prelude::*;
use zosimos::command::Linker;

#[used]
pub static STD: Asset = asset!("/assets/std.cbor");

pub async fn from_assets() -> Result<Linker, Box<dyn std::error::Error>> {
    tracing::info!("Getting shader assets");
    let std = super::asset_to_url(&STD).expect("Missing std shader assets");

    let response = reqwest::get(std).await?;
    let bytes = response.bytes().await?;

    let (core, std) = serde_cbor::from_slice(&bytes)?;
    Ok(Linker { core, std })
}
