use color_eyre::Result;

mod config;
mod rails;
mod scraper;
mod storage;

fn main() -> Result<()> {
    color_eyre::install()?;
    let items = scraper::scrape()?;
    let old_snap = storage::load_latest_snapshot()?;
    match old_snap {
        None => storage::write_new_snapshot(items)?,
        _ => todo!(),
    }
    Ok(())
}
