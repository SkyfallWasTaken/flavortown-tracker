use std::collections::HashMap;

use crate::config::CONFIG;
use color_eyre::{Result, eyre::eyre};
use once_cell::sync::Lazy;
use reqwest::blocking::Client;
use reqwest::{StatusCode, Url, header};
use scraper::{Html, Selector};
use strum::VariantArray;
use strum_macros::{Display, VariantArray};

static CLIENT: Lazy<Client> = Lazy::new(|| {
    let mut headers = header::HeaderMap::new();
    headers.insert(header::COOKIE, CONFIG.cookie.parse().unwrap());
    Client::builder()
        .user_agent(&CONFIG.user_agent)
        .default_headers(headers)
        .build()
        .expect("Failed to build scraping client")
});

#[derive(Display, Debug, VariantArray, Clone)]
pub enum Region {
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
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnitedStates => "US",
            Self::Europe => "EU",
            Self::UnitedKingdom => "UK",
            Self::India => "IN",
            Self::Canada => "CA",
            Self::Australia => "AU",
            Self::Global => "XX",
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

fn scrape_region(region: &Region, csrf_token: &String) -> Result<ShopItems> {
    let mut params = HashMap::new();
    params.insert("region", region.code());
    CLIENT
        .patch("https://flavortown.hackclub.com/shop/update_region")
        .header("X-CSRF-Token", csrf_token)
        .form(&params)
        .send()?
        .error_for_status()?;

    let res = CLIENT
        .get("https://flavortown.hackclub.com/shop")
        .send()?
        .error_for_status()?;
    assert_eq!(res.status(), StatusCode::OK);
    let html = res.text()?;
    let document = Html::parse_document(&html);

    let selector = Selector::parse(".shop-item-card").unwrap();
    let mut items = Vec::new();
    for element in document.select(&selector) {
        let title = select_one(&element, "h4")?.inner_html();
        let description = select_one(&element, "p.shop-item-card__description")?.inner_html();
        let price: u32 = select_one(&element, "span.shop-item-card__price")?
            .text()
            .collect::<String>()
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()?;
        let image_url: Url = select_one(&element, "div.shop-item-card__image > img")?
            .attr("src")
            .ok_or_else(|| eyre!("missing image src"))?
            .parse()?;

        let href: Url = select_one(&element, "div.shop-item-card__order-button > a.btn")?
            .attr("href")
            .ok_or_else(|| eyre!("missing shop order button's url"))?
            .parse()?;

        let shop_item_id: ShopItemId = href
            .query_pairs()
            .find_map(|(k, v)| {
                if k == "shop_item_id" {
                    v.parse().ok()
                } else {
                    None
                }
            })
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

    let res = CLIENT
        .get("https://flavortown.hackclub.com/shop")
        .send()?
        .error_for_status()?;
    assert_eq!(res.status(), StatusCode::OK);
    let html = res.text()?;
    let document = Html::parse_document(&html);
    let selector = Selector::parse("meta[name=\"csrf-token\"]").unwrap();
    let csrf_token = document
        .select(&selector)
        .next()
        .ok_or_else(|| eyre!("Failed to find csrf-token"))?
        .attr("content")
        .unwrap()
        .parse::<String>()
        .unwrap();

    for region in Region::VARIANTS {
        let region_items = scrape_region(region, &csrf_token)?;

        for item in region_items {
            items
                .entry(item.id)
                .and_modify(|e| e.regions.push(region.clone()))
                .or_insert_with(|| {
                    let mut new_item = item;
                    new_item.regions = vec![region.clone()];
                    new_item
                });
        }
    }

    Ok(items.into_values().collect())
}

fn select_one<'a>(
    element: &'a scraper::ElementRef,
    selector: &str,
) -> Result<scraper::ElementRef<'a>> {
    element
        .select(&Selector::parse(selector).unwrap())
        .next()
        .ok_or_else(|| eyre!("missing element: {}", selector))
}
