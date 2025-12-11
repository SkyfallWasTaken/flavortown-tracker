use base64::prelude::*;
use color_eyre::{Result, eyre::eyre};
use reqwest::Url;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct BlobInfo {
    #[serde(rename = "_rails")]
    rails: RailsData,
}

#[derive(Deserialize, Debug)]
struct RailsData {
    #[serde(rename = "data")]
    blob_id: usize,
}

pub fn get_rails_blob_id(url: &Url) -> Result<usize> {
    let s3_info = url
        .path()
        .split('/')
        .rev()
        .nth(2)
        .ok_or_else(|| eyre!("can't find raw s3"))?;
    let blob_info_b64 = s3_info
        .split("--")
        .next()
        .ok_or_else(|| eyre!("can't find the blob info"))?;
    let blob_info_bytes = BASE64_STANDARD.decode(blob_info_b64)?;
    let blob_info_string = String::from_utf8(blob_info_bytes)?;
    let blob_info: BlobInfo = serde_json::from_str(&blob_info_string)?;
    Ok(blob_info.rails.blob_id)
}
