use std::collections::HashMap;

use crate::config::CONFIG;
use crate::scraper::{Region, ShopItem, ShopItems};
use color_eyre::Result;
use log::{debug, info};
use slack_morphism::prelude::*;
use strum::VariantArray;

const EMOJI_SHELLS: &str = ":shells:";
const EMOJI_TROLLEY: &str = ":tw_shopping_trolley:";
const EMOJI_NEW: &str = ":new:";
const EMOJI_TRASH: &str = ":win10-trash:";
const EMOJI_STAR: &str = ":star:";
const EMOJI_ROBOT: &str = ":robot_face:";

fn prices_changed(old: &HashMap<Region, u32>, new: &HashMap<Region, u32>) -> bool {
    old.len() != new.len() || old.iter().any(|(r, p)| new.get(r) != Some(p))
}

fn escape_markdown(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            '_' | '*' | '~' | '`' => vec!['\\', c],
            _ => vec![c],
        })
        .collect()
}

fn format_prices_with_flags(prices: &HashMap<Region, u32>) -> String {
    let price_entries: Vec<_> = prices.iter().collect();

    match price_entries.as_slice() {
        [(region, price)] => format!("{} {price}", region.flag()),
        entries
            if entries.len() == Region::VARIANTS.len()
                && entries.iter().all(|(_, p)| **p == *entries[0].1) =>
        {
            format!(":earth_americas: {}", entries[0].1)
        }
        entries => entries
            .iter()
            .map(|(r, p)| format!("{} {p}", r.flag()))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn item_header(emoji: &str, item: &ShopItem, prices: &HashMap<Region, u32>) -> String {
    format!(
        "{emoji} {} ({EMOJI_SHELLS} {})",
        item.title,
        format_prices_with_flags(prices)
    )
}

fn item_description(desc: &str) -> String {
    if desc.is_empty() {
        String::new()
    } else {
        format!("_{}_\n", escape_markdown(desc))
    }
}

fn buy_button(url: &impl ToString) -> String {
    format!("<{}|*{EMOJI_TROLLEY} Buy*>", url.to_string())
}

fn render_new_item(item: &ShopItem) -> Vec<SlackBlock> {
    let section_text = format!(
        "{}*Stock:* Unlimited\n\n{}",
        item_description(&item.description),
        buy_button(&item.buy_link())
    );

    vec![
        SlackHeaderBlock::new(pt!(item_header(EMOJI_NEW, item, &item.prices))).into(),
        SlackSectionBlock::new().with_text(md!(section_text)).into(),
        SlackImageBlock::new(
            item.image_url.clone().into(),
            format!("Image for {}", item.title),
        )
        .into(),
    ]
}

fn render_deleted_item(item: &ShopItem) -> Vec<SlackBlock> {
    vec![
        SlackHeaderBlock::new(pt!(item_header(EMOJI_TRASH, item, &item.prices))).into(),
        SlackSectionBlock::new()
            .with_text(md!(item_description(&item.description)))
            .into(),
        SlackImageBlock::new(
            item.image_url.clone().into(),
            format!("Image for {}", item.title),
        )
        .into(),
    ]
}

fn render_updated_item(old: &ShopItem, new: &ShopItem) -> Vec<SlackBlock> {
    let title = if old.title != new.title {
        format!("{} → {}", old.title, new.title)
    } else {
        new.title.clone()
    };

    let price = if prices_changed(&old.prices, &new.prices) {
        format!(
            "{} → {}",
            format_prices_with_flags(&old.prices),
            format_prices_with_flags(&new.prices)
        )
    } else {
        format_prices_with_flags(&new.prices)
    };

    let description = match (old.description.is_empty(), new.description.is_empty()) {
        (true, true) => String::new(),
        (false, false) if old.description == new.description => item_description(&new.description),
        _ => {
            let old_desc = if old.description.is_empty() {
                "_no description_"
            } else {
                &escape_markdown(&old.description)
            };
            let new_desc = if new.description.is_empty() {
                "_no description_"
            } else {
                &escape_markdown(&new.description)
            };
            format!("{old_desc} → {new_desc}\n")
        }
    };

    let section_text = format!(
        "{description}*Stock:* Unlimited\n\n{}",
        buy_button(&new.buy_link())
    );

    let mut blocks = vec![
        SlackHeaderBlock::new(pt!(format!("{title} ({EMOJI_SHELLS} {price})"))).into(),
        SlackSectionBlock::new().with_text(md!(section_text)).into(),
    ];

    if old.image_url != new.image_url {
        blocks.push(
            SlackImageBlock::new(
                old.image_url.clone().into(),
                format!("Old image for {}", new.title),
            )
            .into(),
        );
    }

    blocks.push(
        SlackImageBlock::new(
            new.image_url.clone().into(),
            format!("New image for {}", new.title),
        )
        .into(),
    );
    blocks
}

fn render_channel_ping() -> Vec<SlackBlock> {
    vec![SlackContextBlock::new(vec![SlackContextBlockElement::MarkDown(md!(format!(
        "pinging <!channel> · <https://github.com/skyfallwastaken/flavortown-tracker|{EMOJI_STAR} star the repo!> · <https://hackclub.slack.com/archives/C091UF79VDM|{EMOJI_ROBOT} discord/slackbot ysws>"
    )))]).into()]
}

#[derive(Debug)]
pub struct ItemDiff {
    pub new_items: Vec<ShopItem>,
    pub deleted_items: Vec<ShopItem>,
    pub updated_items: Vec<(ShopItem, ShopItem)>,
}

impl ItemDiff {
    pub const fn is_empty(&self) -> bool {
        self.new_items.is_empty() && self.deleted_items.is_empty() && self.updated_items.is_empty()
    }
}

pub fn compute_diff(old_items: &ShopItems, new_items: &ShopItems) -> ItemDiff {
    let old_map: HashMap<_, _> = old_items.iter().map(|i| (i.id, i)).collect();
    let new_map: HashMap<_, _> = new_items.iter().map(|i| (i.id, i)).collect();

    let mut diff = ItemDiff {
        new_items: new_items
            .iter()
            .filter(|item| !old_map.contains_key(&item.id))
            .cloned()
            .collect(),
        deleted_items: old_items
            .iter()
            .filter(|item| !new_map.contains_key(&item.id))
            .cloned()
            .collect(),
        updated_items: Vec::new(),
    };

    diff.updated_items = new_items
        .iter()
        .filter_map(|new_item| {
            old_map
                .get(&new_item.id)
                .filter(|&&old_item| old_item != new_item)
                .map(|old_item| ((*old_item).clone(), new_item.clone()))
        })
        .collect();

    diff
}

const MAX_BLOCKS_PER_MESSAGE: usize = 50;

fn send_blocks(blocks: Vec<SlackBlock>, fallback_text: &str) -> Result<()> {
    use crate::scraper::CLIENT;

    let payload = SlackMessageContent::new()
        .with_text(fallback_text.to_string())
        .with_blocks(blocks);

    debug!(
        "Sending payload: {}",
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );

    let response = CLIENT
        .post(CONFIG.webhook_url.clone())
        .json(&payload)
        .send()?;

    let status = response.status();
    let body = response.text().unwrap_or_default();
    debug!("Response status: {}, body: {}", status, body);
    if !status.is_success() {
        return Err(color_eyre::eyre::eyre!("Slack API error {}: {}", status, body));
    }

    Ok(())
}

pub fn send_webhook_notifications(diff: &ItemDiff) -> Result<()> {
    let mut item_block_groups: Vec<Vec<SlackBlock>> = Vec::new();

    for item in &diff.new_items {
        info!("Sending notification for new item: {}", item.title);
        item_block_groups.push(render_new_item(item));
    }

    for (old_item, new_item) in &diff.updated_items {
        info!("Sending notification for updated item: {}", new_item.title);
        item_block_groups.push(render_updated_item(old_item, new_item));
    }

    for item in &diff.deleted_items {
        info!("Sending notification for deleted item: {}", item.title);
        item_block_groups.push(render_deleted_item(item));
    }

    let fallback_text = format!(
        "Shop update: {} new, {} updated, {} removed",
        diff.new_items.len(),
        diff.updated_items.len(),
        diff.deleted_items.len()
    );

    let mut current_blocks: Vec<SlackBlock> = Vec::new();

    for (i, group) in item_block_groups.into_iter().enumerate() {
        let group_size = group.len() + 1; // +1 for divider

        if !current_blocks.is_empty()
            && current_blocks.len() + group_size > MAX_BLOCKS_PER_MESSAGE - 1
        {
            send_blocks(current_blocks, &fallback_text)?;
            current_blocks = Vec::new();
        }

        current_blocks.extend(group);
        if i < diff.new_items.len() + diff.updated_items.len() + diff.deleted_items.len() - 1 {
            current_blocks.push(SlackDividerBlock::new().into());
        }
    }

    current_blocks.extend(render_channel_ping());
    send_blocks(current_blocks, &fallback_text)?;

    info!("Successfully sent webhook notifications");
    Ok(())
}
