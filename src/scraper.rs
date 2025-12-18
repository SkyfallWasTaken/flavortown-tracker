use std::collections::HashMap;
use std::hash::Hash;

use crate::config::CONFIG;
use crate::storage::{CDN_CACHE_DB, upload_to_cdn};
use color_eyre::{Result, eyre::eyre};
use log::debug;
use once_cell::sync::Lazy;
use rayon::prelude::*;
use reqwest::blocking::Client;
use reqwest::{StatusCode, Url, header, redirect};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use strum::VariantArray;
use strum_macros::{Display, VariantArray};

pub static CLIENT: Lazy<Client> = Lazy::new(|| {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::COOKIE,
        CONFIG
            .cookie
            .parse()
            .expect("cookie parsing failed - check your COOKIE env var"),
    );
    Client::builder()
        .user_agent(&CONFIG.user_agent)
        .default_headers(headers)
        .redirect(redirect::Policy::none())
        .build()
        .expect("failed to build scraping client")
});

#[derive(Display, Debug, VariantArray, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
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

    pub const fn flag(&self) -> &'static str {
        match self {
            Self::UnitedStates => ":flag-us:",
            Self::Europe => ":flag-eu:",
            Self::UnitedKingdom => ":flag-gb:",
            Self::India => ":flag-in:",
            Self::Canada => ":flag-ca:",
            Self::Australia => ":flag-au:",
            Self::Global => ":earth_americas:",
        }
    }
}

pub type ShopItems = Vec<ShopItem>;
pub type ShopItemId = usize;

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ShopItem {
    pub title: String,
    pub description: String,
    pub prices: HashMap<Region, u32>,
    pub image_url: Url,

    pub image_id: usize,
    pub id: ShopItemId,
}

impl ShopItem {
    pub fn buy_link(&self) -> Url {
        let mut url = CONFIG.base_url.join("shop/order").unwrap();
        url.set_query(Some(format!("shop_item_id={}", self.id).as_str()));
        url
    }
}

fn select_one<'a>(element: &'a ElementRef, selector: &str) -> Result<ElementRef<'a>> {
    element
        .select(&Selector::parse(selector).unwrap())
        .next()
        .ok_or_else(|| eyre!("missing element: {}", selector))
}

fn parse_shop_item(element: ElementRef, region: &Region) -> Result<ShopItem> {
    let title = select_one(&element, "h4")?.inner_html();
    let description = select_one(&element, "div.shop-item-card__description > p")?.inner_html();
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
    let image_id = crate::rails::get_rails_blob_id(&image_url)?;
    let id = element
        .attr("data-shop-id")
        .ok_or_else(|| eyre!("missing item id"))?
        .parse()?;

    let mut prices = HashMap::new();
    prices.insert(region.clone(), price);

    Ok(ShopItem {
        title,
        description,
        id,
        image_url,
        image_id,
        prices,
    })
}

fn fetch_shop_page() -> Result<String> {
    let res = CLIENT
        .get(CONFIG.base_url.join("shop")?)
        .send()?
        .error_for_status()?;
    assert_eq!(res.status(), StatusCode::OK);
    res.text().map_err(Into::into)
}

fn get_csrf_token() -> Result<String> {
    let document = Html::parse_document(&fetch_shop_page()?);
    document
        .select(&Selector::parse("meta[name=\"csrf-token\"]").unwrap())
        .next()
        .and_then(|e| e.attr("content"))
        .map(String::from)
        .ok_or_else(|| eyre!("Failed to find csrf-token"))
}

fn set_region(region: &Region, csrf_token: &str) -> Result<()> {
    let res = CLIENT
        .patch(CONFIG.base_url.join("shop/update_region")?)
        .header("X-CSRF-Token", csrf_token)
        .form(&[("region", region.code())])
        .send()?
        .error_for_status()?;
    assert_eq!(res.status(), StatusCode::OK);
    Ok(())
}

fn scrape_region(region: &Region, csrf_token: &str) -> Result<ShopItems> {
    set_region(region, csrf_token)?;

    let document = Html::parse_document(&fetch_shop_page()?);
    let root = document.root_element();

    // step 1: region selection
    let selected_region = select_one(
        &root,
        "button.dropdown__button > span.dropdown__selected > span.dropdown__char-span",
    )?
    .text()
    .next()
    .unwrap();
    assert_eq!(selected_region, region.to_string());

    // step 2: parse all shop items
    document
        .select(&Selector::parse(".shop-item-card").unwrap())
        .map(|element_ref| parse_shop_item(element_ref, region))
        .collect()
}

pub fn scrape() -> Result<Vec<ShopItem>> {
    let mut items: HashMap<ShopItemId, ShopItem> = HashMap::new();
    let csrf_token = get_csrf_token()?;

    for region in Region::VARIANTS {
        debug!("Now scraping {:?}", region);
        for item in scrape_region(region, &csrf_token)? {
            items
                .entry(item.id)
                .and_modify(|e| {
                    e.prices.insert(region.clone(), item.prices[region]);
                })
                .or_insert(item);
        }
    }

    items
        .par_iter_mut()
        .try_for_each(|(_, item)| -> Result<()> {
            item.image_url = upload_to_cdn(item.image_id, &item.image_url.clone())?;
            Ok(())
        })?;

    CDN_CACHE_DB.flush()?;

    let mut items = items.into_values().collect::<ShopItems>();
    items.sort_by_key(|item| item.id);
    Ok(items)
}
