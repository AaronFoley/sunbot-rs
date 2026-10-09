use crate::{Data, Error};
use poise::serenity_prelude as serenity;
use sea_orm::*;
use sunbot_db::entities::prelude::*;

use tracing::info;

mod dad;
pub mod lavalink;
mod openai;
pub(crate) mod punish;

pub async fn handler(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    framework: poise::FrameworkContext<'_, Data, Error>,
) -> Result<(), Error> {
    match event {
        serenity::FullEvent::Message { new_message } => {
            dad::handle_message(ctx, framework, new_message).await?;
            openai::handle_random_message(ctx, framework, new_message).await?;
            openai::handle_reply(ctx, framework, new_message).await?;
        }
        serenity::FullEvent::GuildCreate { guild, .. } => {
            info!("Joined Guild {}: {}", guild.id, guild.name);
            let guild = sunbot_db::entities::guild::ActiveModel {
                id: ActiveValue::Set(guild.id.get() as i64),
                ..Default::default()
            };
            Guild::insert(guild)
                .on_conflict_do_nothing()
                .exec(framework.user_data.db)
                .await?;
        }
        serenity::FullEvent::VoiceStateUpdate { old, new, .. } => {
            punish::on_voice_state_change(ctx, framework, old, new).await?;
        }
        _ => {}
    }

    Ok(())
}
