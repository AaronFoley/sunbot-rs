use crate::{Data, Error};
use lazy_static::lazy_static;
use poise::serenity_prelude as serenity;
use rand::RngExt;
use regex::{Regex, RegexBuilder};

lazy_static! {
    static ref PATTERN: Regex = RegexBuilder::new(r"\bi(?:'| +a|’)?m +([\w\- ]*)")
        .case_insensitive(true)
        .build()
        .unwrap();
}

pub async fn handle_message(
    ctx: &serenity::Context,
    framework: poise::FrameworkContext<'_, Data, Error>,
    message: &serenity::Message,
) -> Result<(), Error> {
    // If from a bot or an empty message
    if message.author.bot || message.content.is_empty() {
        return Ok(());
    }

    if let Some(caps) = PATTERN.captures(&message.content) {
        if let Some(name) = caps.get(1) {
            if name.as_str().len() > 32 {
                return Ok(());
            }
            if rand::rng().random::<f64>() < 0.8 {
                let bot_user = framework.bot_id().to_user(&ctx.http).await?;
                message
                    .reply_ping(
                        &ctx.http,
                        format!("Hi {}, I'm {}", name.as_str(), bot_user.name),
                    )
                    .await?;

                if let Some(guild) = message.guild_id {
                    let builder = serenity::EditMember::new().nickname(name.as_str());
                    guild
                        .edit_member(&ctx.http, &message.author, builder)
                        .await?;
                }
            }
        }
    }

    Ok(())
}
