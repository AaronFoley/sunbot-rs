use crate::constants::ERROR_COLOUR;
use crate::Context;
use poise::serenity_prelude as serenity;

use tracing::{error, info, warn};

// Check if a message is a reply or a mention for a specific user
pub async fn is_reply_or_mention(
    ctx: &serenity::Context,
    message: &serenity::Message,
    user_id: serenity::UserId,
) -> bool {
    if let Some(ref reply) = message.message_reference {
        if let Some(msg_id) = reply.message_id {
            if let Ok(msg) = ctx.http.get_message(reply.channel_id, msg_id).await {
                if msg.author.id == user_id {
                    info!("Reply detected: {}", msg.content);
                    return true;
                }
            }
        }
    }

    if message.mentions_user_id(user_id) {
        info!("Mention detected: {}", message.content);
        return true;
    }

    false
}

/// Reply with an error message
pub async fn send_err_msg(ctx: Context<'_>, title: &str, description: &str, ephemeral: bool) {
    let embed = serenity::CreateEmbed::default()
        .title(title)
        .color(ERROR_COLOUR)
        .description(description);
    let resp = ctx
        .send(
            poise::CreateReply::default()
                .ephemeral(ephemeral)
                .embed(embed.clone()),
        )
        .await;

    if let Err(e) = resp {
        warn!("Failed to send message while handling error: {}", e);
        let resp = ctx
            .author()
            .direct_message(&ctx.http(), serenity::CreateMessage::default().embed(embed))
            .await;

        if let Err(e) = resp {
            error!("Failed to send DM while handling error: {}", e);
        }
    }
}
