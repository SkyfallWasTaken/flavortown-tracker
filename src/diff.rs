use std::collections::HashMap;

use crate::config::CONFIG;
use crate::scraper::{Region, ShopItem, ShopItems};
use color_eyre::Result;
use log::info;
use slack_morphism::prelude::*;
use strum::VariantArray;

const EMOJI_SHELLS: &str = ":shells:";
const EMOJI_TROLLEY: &str = ":tw_shopping_trolley:";
const EMOJI_NEW: &str = ":new:";
const EMOJI_TRASH: &str = ":win10-trash:";
const EMOJI_STAR: &str = ":star:";
const EMOJI_ROBOT: &str = ":robot_face:";

fn format_prices(prices: &HashMap<Region, u32>) -> String {
    let price_entries: Vec<_> = prices.iter().collect();

    match price_entries.as_slice() {
        [(region, price)] => format!("{price} ({region})"),
        entries
            if entries.len() == Region::VARIANTS.len()
                && entries.iter().all(|(_, p)| **p == *entries[0].1) =>
        {
            format!("{} (Rest of World)", entries[0].1)
        }
        entries => entries
            .iter()
            .map(|(r, p)| format!("{r} {p}"))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

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

fn item_header(emoji: &str, item: &ShopItem, prices: &HashMap<Region, u32>) -> String {
    format!(
        "{emoji} {} ({EMOJI_SHELLS} {})",
        item.title,
        format_prices(prices)
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
            format_prices(&old.prices),
            format_prices(&new.prices)
        )
    } else {
        format_prices(&new.prices)
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
        // Use context block to display old and new images side by side
        let old_image = SlackBlockImageElement::new(
            old.image_url.clone().into(),
            format!("Old image for {}", new.title),
        );
        let new_image = SlackBlockImageElement::new(
            new.image_url.clone().into(),
            format!("New image for {}", new.title),
        );
        blocks.push(
            SlackContextBlock::new(vec![
                SlackContextBlockElement::MarkDown(md!("*Old:*")),
                SlackContextBlockElement::Image(old_image),
                SlackContextBlockElement::MarkDown(md!("→ *New:*")),
                SlackContextBlockElement::Image(new_image),
            ])
            .into(),
        );
    } else {
        blocks.push(
            SlackImageBlock::new(
                new.image_url.clone().into(),
                format!("Image for {}", new.title),
            )
            .into(),
        );
    }
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

// Slack's maximum block limit per message
const SLACK_MAX_BLOCKS: usize = 50;

pub fn send_webhook_notifications(diff: &ItemDiff) -> Result<()> {
    use crate::scraper::CLIENT;

    // Collect blocks for each item separately so we can chunk them properly
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

    let summary_text = format!(
        "Shop update: {} new, {} updated, {} removed",
        diff.new_items.len(),
        diff.updated_items.len(),
        diff.deleted_items.len()
    );

    // Chunk item groups into messages that fit within the block limit
    let mut messages: Vec<Vec<SlackBlock>> = Vec::new();
    let mut current_message_blocks: Vec<SlackBlock> = Vec::new();

    for item_blocks in item_block_groups {
        // +1 for divider between items (if not first item in message)
        let blocks_needed = if current_message_blocks.is_empty() {
            item_blocks.len()
        } else {
            item_blocks.len() + 1 // +1 for divider
        };

        // Check if adding this item would exceed the limit
        // Reserve 1 block for channel ping in the last message
        if !current_message_blocks.is_empty()
            && current_message_blocks.len() + blocks_needed > SLACK_MAX_BLOCKS - 1
        {
            messages.push(current_message_blocks);
            current_message_blocks = Vec::new();
        }

        // Add divider between items within the same message
        if !current_message_blocks.is_empty() {
            current_message_blocks.push(SlackDividerBlock::new().into());
        }

        current_message_blocks.extend(item_blocks);
    }

    // Add the remaining blocks as the last message
    if !current_message_blocks.is_empty() {
        messages.push(current_message_blocks);
    }

    // Add channel ping to the last message
    if let Some(last_message) = messages.last_mut() {
        last_message.extend(render_channel_ping());
    }

    // Send all messages
    for (i, blocks) in messages.iter().enumerate() {
        let text = if messages.len() == 1 {
            summary_text.clone()
        } else {
            format!("{} (part {}/{})", summary_text, i + 1, messages.len())
        };

        let payload = SlackMessageContent::new()
            .with_text(text)
            .with_blocks(blocks.clone());

        CLIENT
            .post(CONFIG.webhook_url.clone())
            .json(&payload)
            .send()?
            .error_for_status()?;
    }

    info!("Successfully sent webhook notifications");
    Ok(())
}
