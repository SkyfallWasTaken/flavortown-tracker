use std::collections::HashMap;

use crate::config::CONFIG;
use color_eyre::{Result, eyre::eyre};
use once_cell::sync::Lazy;
use reqwest::blocking::Client;
use reqwest::{Url, header};
use scraper::{Html, Selector};
use strum::VariantArray;
use strum_macros::{Display, VariantArray};

static CLIENT: Lazy<Client> = Lazy::new(|| {
    let mut headers = header::HeaderMap::new();
    headers.insert(header::COOKIE, "".parse().unwrap());
    Client::builder()
        .user_agent(&CONFIG.user_agent)
        .default_headers(headers)
        .build()
        .expect("Failed to build scraping client")
});

#[derive(Display, Debug, VariantArray, Clone)]
enum Region {
    #[strum(to_string = "USA")]
    UnitedStates,

    #[strum(to_string = "Europe")]
    Europe,

    #[strum(to_string = "UK")]
    UnitedKingdom,

    #[strum(to_string = "India")]
    India,

    #[strum(to_string = "Canada")]
    Canada,

    #[strum(to_string = "Australia")]
    Australia,

    #[strum(to_string = "Global")]
    Global,
}

impl Region {
    pub fn code(&self) -> &'static str {
        match self {
            Region::UnitedStates => "US",
            Region::Europe => "EU",
            Region::UnitedKingdom => "UK",
            Region::India => "IN",
            Region::Canada => "CA",
            Region::Australia => "AU",
            Region::Global => "XX",
        }
    }
}

pub type ShopItems = Vec<ShopItem>;
pub type ShopItemId = usize;
#[derive(Debug, Clone)]
pub struct ShopItem {
    pub title: String,
    pub description: String,
    pub price: u32,
    pub image_url: Url,
    pub id: ShopItemId,
    pub regions: Vec<Region>,
}

fn scrape_region(region: &Region) -> Result<ShopItems> {
    let res = CLIENT
        .get("https://flavortown.hackclub.com/shop")
        .query(&[("region", region.code())])
        .send()?
        .error_for_status()?;
    let html = res.text()?;
    let document = Html::parse_document(&html);

    let selector = Selector::parse(".shop-item-card").unwrap();
    let mut items = Vec::new();
    for element in document.select(&selector) {
        let selector = Selector::parse("h4").unwrap();
        let title = element
            .select(&selector)
            .next()
            .ok_or_else(|| eyre!("missing title element"))?
            .inner_html();

        let selector = Selector::parse("p.shop-item-card__description").unwrap();
        let description = element
            .select(&selector)
            .next()
            .ok_or_else(|| eyre!("missing description element"))?
            .inner_html();

        let selector = Selector::parse("span.shop-item-card__price").unwrap();
        let price_text = element
            .select(&selector)
            .next()
            .ok_or_else(|| eyre!("missing price element"))?
            .text()
            .next()
            .unwrap();
        let price: u32 = price_text
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .map_err(|_| eyre!("missing title element"))?;

        let selector = Selector::parse("div.shop-item-card__image > img").unwrap();
        let image_url = element
            .select(&selector)
            .next()
            .ok_or_else(|| eyre!("missing image element"))?
            .attr("src")
            .ok_or_else(|| eyre!("found image element but can't find its source"))?;
        let image_url = Url::parse(image_url).unwrap();

        let selector = Selector::parse("div.shop-item-card__order-button > a.btn").unwrap();
        let href: Url = element
            .select(&selector)
            .next()
            .ok_or_else(|| eyre!("Failed to find shop order button"))?
            .attr("href")
            .ok_or_else(|| eyre!("missing shop order button's url"))?
            .parse()
            .unwrap();
        let shop_item_id: ShopItemId = href
            .query_pairs()
            .find(|(k, _)| k == "shop_item_id")
            .and_then(|(_, v)| v.parse().ok())
            .ok_or_else(|| eyre!("can't find or parse shop item id"))?;

        items.push(ShopItem {
            title,
            description,
            id: shop_item_id,
            price,
            image_url,
            regions: Vec::new(),
        })
    }

    Ok(items)
}

pub fn scrape() -> Result<Vec<ShopItem>> {
    let mut items: HashMap<ShopItemId, ShopItem> = HashMap::new();

    for region in Region::VARIANTS {
        let mut region_items = scrape_region(region)?;

        for item in region_items.drain(..) {
            match items.get_mut(&item.id) {
                Some(existing) => {
                    existing.regions.push(region.clone());
                }
                None => {
                    let mut new_item = item;
                    new_item.regions = vec![region.clone()];
                    items.insert(new_item.id, new_item);
                }
            }
        }
    }

    Ok(items.into_values().collect())
}
