use std::collections::HashMap;

use crate::config::CONFIG;
use color_eyre::{Result, eyre::eyre};
use once_cell::sync::Lazy;
use reqwest::blocking::Client;
use reqwest::{StatusCode, Url, header, redirect};
use scraper::{Html, Selector};
use strum::VariantArray;
use strum_macros::{Display, VariantArray};

static CLIENT: Lazy<Client> = Lazy::new(|| {
    let mut headers = header::HeaderMap::new();
    headers.insert(header::COOKIE, CONFIG.cookie.parse().unwrap());
    Client::builder()
        .user_agent(&CONFIG.user_agent)
        .default_headers(headers)
        .redirect(redirect::Policy::none())
        .build()
        .expect("Failed to build scraping client")
});

#[derive(Display, Debug, VariantArray, Clone)]
pub enum Region {
    #[strum(to_string = "United States")]
    UnitedStates,

    #[strum(to_string = "EU")]
    Europe,

    #[strum(to_string = "United Kingdom")]
    UnitedKingdom,

    #[strum(to_string = "India")]
    India,

    #[strum(to_string = "Canada")]
    Canada,

    #[strum(to_string = "Australia")]
    Australia,

    #[strum(to_string = "Rest of World")]
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
    set_region(region, csrf_token)?;

    let res = CLIENT
        .get(CONFIG.base_url.join("shop")?)
        .send()?
        .error_for_status()?;
    assert_eq!(res.status(), StatusCode::OK);
    let html = res.text()?;
    let document = Html::parse_document(&html);
    let root = document.root_element();

    let selected_region = select_one(
        &root,
        "button.dropdown__button > span.dropdown__selected > span.dropdown__char-span",
    )?
    .text()
    .next()
    .unwrap();
    assert_eq!(selected_region, region.to_string());

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

        let href_part = select_one(&element, "div.shop-item-card__order-button > a.btn")?
            .attr("href")
            .ok_or_else(|| eyre!("missing shop order button's url"))?;
        let href = CONFIG.base_url.join(href_part)?;

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
    let csrf_token = get_csrf_token()?;

    for region in Region::VARIANTS {
        for item in scrape_region(region, &csrf_token)? {
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

fn get_csrf_token() -> Result<String> {
    let html = CLIENT
        .get(CONFIG.base_url.join("shop")?)
        .send()?
        .error_for_status()?
        .text()?;

    let document = Html::parse_document(&html);
    let csrf_token = document
        .select(&Selector::parse("meta[name=\"csrf-token\"]").unwrap())
        .next()
        .and_then(|e| e.attr("content"))
        .ok_or_else(|| eyre!("Failed to find csrf-token"))?;
    Ok(csrf_token.into())
}

fn set_region(region: &Region, csrf_token: &String) -> Result<()> {
    let mut params = HashMap::new();
    params.insert("region", region.code());
    CLIENT
        .patch(CONFIG.base_url.join("shop/update_region")?)
        .header("X-CSRF-Token", csrf_token)
        .form(&params)
        .send()?
        .error_for_status()?;
    Ok(())
}
