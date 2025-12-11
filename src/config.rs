use std::path::PathBuf;

use color_eyre::eyre::Context;
use once_cell::sync::Lazy;
use reqwest::Url;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub cookie: String,
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
    #[serde(default = "default_base_url")]
    pub base_url: Url,
    #[serde(default = "default_storage_path")]
    pub storage_path: PathBuf,
}

fn default_user_agent() -> String {
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36".into()
}

fn default_base_url() -> Url {
    Url::parse("https://flavortown.hackclub.com/").unwrap()
}

fn default_storage_path() -> PathBuf {
    std::env::current_dir().unwrap().join("flavortown-storage")
}

pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    dotenvy::dotenv().ok();
    envy::from_env::<Config>()
        .wrap_err("failed to load config")
        .unwrap()
});
