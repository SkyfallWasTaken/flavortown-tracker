use std::fs::{self, File};
use std::path::Path;

use crate::config::CONFIG;
use crate::scraper::{ShopItems, CLIENT};

use color_eyre::{eyre::eyre, Result};
use dashmap::DashMap;
use log::debug;
use once_cell::sync::Lazy;
use reqwest::{
    blocking::multipart::{Form, Part},
    Url,
};
use serde::Deserialize;
use sled::{Config, Db};
use std::sync::Arc;

const LATEST_SNAPSHOT_POINTER_PATH: &str = "latest-snapshot.ptr";
const CDN_CACHE_PATH: &str = "cdn-cache.sled";

pub fn load_latest_snapshot() -> Result<Option<ShopItems>> {
    match std::fs::read_to_string(CONFIG.storage_path.join(LATEST_SNAPSHOT_POINTER_PATH)) {
        Ok(snap_ptr) => Ok(Some(serde_json::from_reader(File::open(
            CONFIG.storage_path.join(snap_ptr),
        )?)?)),
        Err(_) => Ok(None),
    }
}

pub fn write_new_snapshot(items: ShopItems) -> Result<()> {
    let ts = time_format::now().unwrap();
    let snap_path = format!(
        "snap_{}.json",
        time_format::strftime_utc("%Y-%m-%d-%H:%M:%S", ts).unwrap()
    );
    fs::create_dir_all(&CONFIG.storage_path)?;
    fs::write(
        CONFIG.storage_path.join(&snap_path),
        serde_json::to_string_pretty(&items)?,
    )?;
    fs::write(
        CONFIG.storage_path.join(LATEST_SNAPSHOT_POINTER_PATH),
        snap_path,
    )?;
    Ok(())
}

pub static CDN_CACHE_DB: Lazy<Db> = Lazy::new(|| {
    Config::new()
        .path(CONFIG.storage_path.join(CDN_CACHE_PATH))
        .flush_every_ms(None) // no auto-flushing - we flush ourselves after scraping.
        .open()
        .unwrap()
});

static UPLOAD_ONCE: Lazy<DashMap<usize, Arc<once_cell::sync::OnceCell<Url>>>> =
    Lazy::new(DashMap::new);

#[derive(Deserialize)]
struct CdnResponse {
    url: Url,
}

pub fn upload_to_cdn(image_id: usize, image_url: &Url) -> Result<Url> {
    let key = image_id.to_le_bytes();

    if let Some(cached) = CDN_CACHE_DB.get(key)? {
        let url_str = std::str::from_utf8(&cached)?;
        return Ok(Url::parse(url_str)?);
    }

    // get the cell/lock for this specific image_id.
    debug!("Didn't find {image_url} (blob ID: {image_id}) - uploading to CDN.");
    let cell = UPLOAD_ONCE
        .entry(image_id)
        .or_insert_with(|| Arc::new(once_cell::sync::OnceCell::new()))
        .clone();

    // only runs once per image_id.
    let file = CLIENT.get(image_url.clone()).send()?.bytes()?.to_vec();
    let ext = ext_from_url(image_url).ok_or_else(|| {
        eyre!("when trying to upload {image_url}, I couldn't get the file extension")
    })?;
    let form = Form::new().part("file", Part::bytes(file).file_name(format!("image.{ext}")));

    let cdn_url = cell.get_or_try_init(|| {
        let json: CdnResponse = CLIENT
            .post(&CONFIG.cdn_base_url)
            .multipart(form)
            .bearer_auth(&CONFIG.cdn_key)
            .send()?
            .error_for_status()?
            .json()?;
        CDN_CACHE_DB.insert(key, json.url.as_str().as_bytes())?;
        Ok::<Url, color_eyre::eyre::ErrReport>(json.url)
    })?;

    Ok(cdn_url.clone())
}

fn ext_from_url(url: &Url) -> Option<String> {
    let filename = url.path_segments()?.next_back()?;

    Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_string())
}
