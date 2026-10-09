use std::time::Duration;

use crate::{constants::SUCCESS_COLOUR, utils::send_err_msg, Context, Error};
use humantime::format_duration;
use lavalink_rs::prelude::*;
use poise::serenity_prelude as serenity;
use sea_orm::*;
use std::ops::Deref;
use sunbot_db::entities::{prelude::*, punishment, punishment_channel, punishment_song};

const DEFAULT_PUNISHMENT_DURATION: i64 = 30;

#[poise::command(
    slash_command,
    guild_only,
    subcommands(
        "punish_show",
        "punish_clear",
        "punish_user",
        "punish_channel",
        "punish_song"
    ),
    subcommand_required,
    required_permissions = "MOVE_MEMBERS"
)]
pub async fn punish(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// List all current punishments
#[poise::command(
    slash_command,
    guild_only,
    required_permissions = "MOVE_MEMBERS",
    rename = "show"
)]
pub async fn punish_show(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx
        .guild_id()
        .ok_or_else(|| Error::GuildExpected)?
        .to_owned();

    // Delete any expired punishments
    Punishment::delete_many()
        .filter(punishment::Column::Guild.eq(guild_id.get() as i64))
        .filter(punishment::Column::ExpiresAt.lt(chrono::Utc::now()))
        .exec(ctx.data().db)
        .await?;

    let punishments: Vec<punishment::Model> = Punishment::find()
        .filter(punishment::Column::Guild.eq(guild_id.get() as i64))
        .all(ctx.data().db)
        .await?;

    // Output a nice summary to the user only
    let current_punishments = punishments
        .iter()
        .map(|p| {
            let duration = (p.expires_at - chrono::Utc::now().naive_utc())
                .to_std()
                .unwrap_or(Duration::ZERO);
            format!(
                "- User: <@{}> Expires In: {}",
                p.user,
                format_duration(duration)
            )
        })
        .collect::<Vec<String>>()
        .join("\n");

    let embed = serenity::CreateEmbed::new()
        .title("Current Punishments")
        .color(SUCCESS_COLOUR)
        .description(current_punishments);

    ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
        .await?;

    Ok(())
}

/// Clear all punishments for a user
#[poise::command(
    slash_command,
    guild_only,
    required_permissions = "MOVE_MEMBERS",
    rename = "clear"
)]
pub async fn punish_clear(ctx: Context<'_>, user: Option<serenity::UserId>) -> Result<(), Error> {
    let guild_id = ctx
        .guild_id()
        .ok_or_else(|| Error::GuildExpected)?
        .to_owned();

    // Delete any expired punishments
    Punishment::delete_many()
        .filter(punishment::Column::Guild.eq(guild_id.get() as i64))
        .filter(punishment::Column::ExpiresAt.lt(chrono::Utc::now()))
        .exec(ctx.data().db)
        .await?;

    match user {
        Some(user) => {
            let result = Punishment::delete_many()
                .filter(punishment::Column::Guild.eq(guild_id.get() as i64))
                .filter(punishment::Column::User.eq(user.get() as i64))
                .exec(ctx.data().db)
                .await?;

            let embed = serenity::CreateEmbed::new()
                .title("Deleted Punishments")
                .color(SUCCESS_COLOUR)
                .description(format!(
                    "Deleted {} punishments for <@{}>",
                    result.rows_affected, user
                ));

            ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
                .await?;
        }
        None => {
            // Delete the punishment for the user
            let result = Punishment::delete_many()
                .filter(punishment::Column::Guild.eq(guild_id.get() as i64))
                .exec(ctx.data().db)
                .await?;

            let embed = serenity::CreateEmbed::new()
                .title("Deleted Punishments")
                .color(SUCCESS_COLOUR)
                .description(format!(
                    "Cleared all punishments ({} total)",
                    result.rows_affected
                ));

            ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
                .await?;
        }
    }

    Ok(())
}

/// Add a punishment for a user
#[poise::command(
    slash_command,
    guild_only,
    required_permissions = "MOVE_MEMBERS",
    rename = "user"
)]
pub async fn punish_user(
    ctx: Context<'_>,
    user: serenity::User,
    duration: Option<i64>,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or_else(|| Error::GuildExpected)?;

    // Get the Punishment channel
    let pchannel = match PunishmentChannel::find()
        .filter(punishment_channel::Column::Guild.eq(guild_id.get()))
        .one(ctx.data().db)
        .await?
    {
        Some(pchannel) => pchannel,
        // No Punishment Channel set for this guild
        None => {
            send_err_msg(
                ctx,
                "No Punishment Channel Set",
                "You can set a punishment channel using `/punish channel <channel>`",
                true,
            )
            .await;
            return Ok(());
        }
    };

    let duration = duration.unwrap_or(DEFAULT_PUNISHMENT_DURATION);
    if duration <= 0
        || chrono::Utc::now()
            .checked_add_signed(
                chrono::Duration::try_seconds(duration).ok_or(Error::InvalidDuration)?,
            )
            .is_none()
    {
        return Err(Error::InvalidDuration);
    }

    // Check for an existing punishment
    let existing_punishment: Option<punishment::Model> = Punishment::find()
        .filter(punishment::Column::Guild.eq(guild_id.get() as i64))
        .filter(punishment::Column::User.eq(user.id.get() as i64))
        .one(ctx.data().db)
        .await?;

    match existing_punishment {
        Some(punishment) => {
            // Update existing punishment for new duration
            let mut punishment: punishment::ActiveModel = punishment.into();
            punishment.expires_at =
                Set((chrono::Utc::now() + chrono::Duration::seconds(duration)).naive_utc());
            punishment.update(ctx.data().db).await?;

            let embed = serenity::CreateEmbed::new()
                .title("Updated Punishment")
                .color(SUCCESS_COLOUR)
                .description(format!(
                    "Updated punishment for <@{}> for {}",
                    user,
                    format_duration(Duration::from_secs(duration as u64))
                ));

            ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
                .await?;
        }
        None => {
            let punishment = punishment::ActiveModel {
                guild: Set(guild_id.get() as i64),
                user: Set(user.id.get() as i64),
                expires_at: Set(
                    (chrono::Utc::now() + chrono::Duration::seconds(duration)).naive_utc()
                ),
                ..Default::default()
            };
            punishment.insert(ctx.data().db).await?;

            let embed = serenity::CreateEmbed::new()
                .title("Added Punishment")
                .color(SUCCESS_COLOUR)
                .description(format!(
                    "Added punishment for <@{}> for {} seconds",
                    user.id, duration
                ));

            ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
                .await?;
        }
    }

    let guild = ctx.guild().unwrap().deref().clone();
    let user_channel_id = guild
        .voice_states
        .get(&user.id)
        .and_then(|voice_state| voice_state.channel_id);

    if let Some(user_channel_id) = user_channel_id {
        if user_channel_id != pchannel.channel as u64 {
            let builder = serenity::EditMember::new().voice_channel(pchannel.channel as u64);
            guild.id.edit_member(&ctx.http(), &user.id, builder).await?;
        }
    }

    Ok(())
}

/// View/set the punishment channel
#[poise::command(
    slash_command,
    guild_only,
    required_permissions = "MOVE_MEMBERS",
    rename = "channel"
)]
pub async fn punish_channel(
    ctx: Context<'_>,
    #[channel_types("Voice")] channel: Option<serenity::GuildChannel>,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or_else(|| Error::GuildExpected)?;

    match channel {
        Some(channel) => {
            ctx.defer_ephemeral().await?;
            let lock = crate::handlers::punish::audio_lock(guild_id).await;
            let _guard = lock.lock().await;
            let previous = PunishmentChannel::find()
                .filter(punishment_channel::Column::Guild.eq(guild_id.get() as i64))
                .one(ctx.data().db)
                .await?;
            if previous.is_some_and(|previous| previous.channel as u64 != channel.id.get()) {
                if let Some(client) = ctx.data().lavalink.as_ref() {
                    crate::handlers::punish::disconnect_punishment(
                        ctx.serenity_context(),
                        client,
                        guild_id,
                        false,
                    )
                    .await?;
                }
            }
            let punish_channel = punishment_channel::ActiveModel {
                guild: Set(guild_id.get() as i64),
                channel: Set(channel.id.get() as i64),
                ..Default::default()
            };

            punishment_channel::Entity::insert(punish_channel)
                .on_conflict(
                    sea_query::OnConflict::columns([punishment_channel::Column::Guild])
                        .update_column(punishment_channel::Column::Channel)
                        .to_owned(),
                )
                .exec(ctx.data().db)
                .await?;

            if let Some(client) = ctx.data().lavalink.as_ref() {
                crate::handlers::punish::start_punishment(
                    ctx.serenity_context(),
                    client,
                    guild_id,
                    channel.id,
                )
                .await?;
            }
            let embed = serenity::CreateEmbed::new()
                .title("Punishment Channel Set")
                .color(SUCCESS_COLOUR)
                .description(format!("Set punishment channel to <#{}>", channel));

            ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
                .await?;
        }
        None => {
            let punish_channel = PunishmentChannel::find()
                .filter(punishment_channel::Column::Guild.eq(guild_id.get() as i64))
                .one(ctx.data().db)
                .await?;

            match punish_channel {
                Some(punish_channel) => {
                    let embed = serenity::CreateEmbed::new()
                        .title("Punishment Channel")
                        .color(SUCCESS_COLOUR)
                        .description(format!(
                            "Punishment channel is set to <#{}>",
                            punish_channel.channel
                        ));

                    ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
                        .await?;
                }
                None => {
                    send_err_msg(
                        ctx,
                        "No punishment channel set",
                        "You can set a punishment channel using `/punish channel <channel>`",
                        true,
                    )
                    .await;
                }
            }
        }
    }

    Ok(())
}

#[poise::command(
    slash_command,
    guild_only,
    subcommands("punish_song_list", "punish_song_add", "punish_song_remove"),
    subcommand_required,
    required_permissions = "MOVE_MEMBERS",
    rename = "song"
)]
pub async fn punish_song(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// List songs that are configured for the punishment channel
#[poise::command(
    slash_command,
    guild_only,
    required_permissions = "MOVE_MEMBERS",
    rename = "list"
)]
pub async fn punish_song_list(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or_else(|| Error::GuildExpected)?;

    let songs = PunishmentSong::find()
        .filter(punishment_song::Column::Guild.eq(guild_id.get() as i64))
        .all(ctx.data().db)
        .await?;

    let song_list = songs
        .iter()
        .map(|s| format!("- [{}]({})", s.name, s.url))
        .collect::<Vec<String>>()
        .join("\n");

    let embed = serenity::CreateEmbed::new()
        .title("Punishment Songs")
        .color(SUCCESS_COLOUR)
        .description(song_list);

    ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
        .await?;

    Ok(())
}

/// Add a song to the punishment channel
#[poise::command(
    slash_command,
    guild_only,
    required_permissions = "MOVE_MEMBERS",
    rename = "add"
)]
pub async fn punish_song_add(
    ctx: Context<'_>,
    #[description = "Search term or URL"]
    #[rest]
    term: String,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or_else(|| Error::GuildExpected)?;

    let lava_client = match &ctx.data().lavalink {
        Some(x) => x,
        None => {
            send_err_msg(ctx, "Error", "Lavalink client is not available.", false).await;
            return Ok(());
        }
    };

    let query = if term.starts_with("http") {
        term
    } else {
        SearchEngines::YouTube.to_query(&term)?
    };

    ctx.defer_ephemeral().await?;
    let loaded_tracks = lava_client.load_tracks(guild_id, &query).await?;
    let track: TrackInQueue = match loaded_tracks.data {
        Some(TrackLoadData::Track(x)) => x.into(),
        Some(TrackLoadData::Search(x)) => match super::music::first_search_track(x) {
            Some(track) => track.into(),
            None => {
                send_err_msg(
                    ctx,
                    "No tracks found",
                    "Try a different search or URL.",
                    true,
                )
                .await;
                return Ok(());
            }
        },
        Some(TrackLoadData::Playlist(_)) => {
            send_err_msg(
                ctx,
                "Playlists not supported",
                "Playlists are not supported for this command",
                false,
            )
            .await;
            return Ok(());
        }
        Some(TrackLoadData::Error(x)) => {
            send_err_msg(ctx, "Error", x.message.as_str(), false).await;
            return Ok(());
        }
        _ => {
            send_err_msg(ctx, "Error", format!("{:?}", loaded_tracks).as_str(), false).await;
            return Ok(());
        }
    };

    // Skip tracks with no url
    if track.track.info.uri.is_none() {
        send_err_msg(ctx, "Error", "No URL found for this track", false).await;
        return Ok(());
    }

    // Check if URL is already in database
    let existing_song: Option<punishment_song::Model> = PunishmentSong::find()
        .filter(punishment_song::Column::Guild.eq(guild_id.get() as i64))
        .filter(
            punishment_song::Column::Url.eq(track
                .track
                .info
                .uri
                .clone()
                .unwrap_or_default()
                .as_str()),
        )
        .one(ctx.data().db)
        .await?;

    if existing_song.is_some() {
        send_err_msg(
            ctx,
            "Song already exists",
            "This song is already defined",
            false,
        )
        .await;
        return Ok(());
    }

    // create the song entry
    let song = punishment_song::ActiveModel {
        guild: Set(guild_id.get() as i64),
        url: Set(track.track.info.uri.to_owned().unwrap_or_default()),
        name: Set(track.track.info.title.to_owned().to_lowercase()),
        ..Default::default()
    };
    punishment_song::Entity::insert(song)
        .on_conflict(
            sea_query::OnConflict::columns([
                punishment_song::Column::Guild,
                punishment_song::Column::Url,
            ])
            .update_column(punishment_song::Column::Name)
            .to_owned(),
        )
        .exec(ctx.data().db)
        .await?;

    let embed = serenity::CreateEmbed::new()
        .title("Added Punishment Song")
        .color(SUCCESS_COLOUR)
        .description(format!(
            "Added punishment song [{}](<{}>)",
            track.track.info.title,
            track.track.info.uri.unwrap_or_default()
        ));

    ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
        .await?;

    Ok(())
}

async fn autocomplete_song_name(
    ctx: Context<'_>,
    partial: &str,
) -> serenity::CreateAutocompleteResponse {
    let songs = match ctx.guild_id() {
        Some(guild_id) => PunishmentSong::find()
            .filter(punishment_song::Column::Guild.eq(guild_id.get() as i64))
            .filter(punishment_song::Column::Name.like(format!("%{}%", partial.to_lowercase())))
            .all(ctx.data().db)
            .await
            .unwrap_or_default()
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<String>>(),
        None => vec![],
    };
    songs.into_iter().take(25).fold(
        serenity::CreateAutocompleteResponse::new(),
        |response, name| response.add_string_choice(name.clone(), name),
    )
}

/// Remove a song from the punishment channel
#[poise::command(
    slash_command,
    guild_only,
    required_permissions = "MOVE_MEMBERS",
    rename = "remove"
)]
pub async fn punish_song_remove(
    ctx: Context<'_>,
    #[autocomplete = "autocomplete_song_name"] name: String,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or_else(|| Error::GuildExpected)?;

    // Get the song from the database
    let song = PunishmentSong::find()
        .filter(punishment_song::Column::Guild.eq(guild_id.get() as i64))
        .filter(punishment_song::Column::Name.eq(name.to_lowercase()))
        .one(ctx.data().db)
        .await?;

    match song {
        Some(song) => {
            PunishmentSong::delete(song.clone().into_active_model())
                .exec(ctx.data().db)
                .await?;

            let embed = serenity::CreateEmbed::new()
                .title("Removed Punishment Song")
                .color(SUCCESS_COLOUR)
                .description(format!(
                    "Removed punishment song [{}]({})",
                    song.name, song.url
                ));

            ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
                .await?;
        }
        None => {
            send_err_msg(ctx, "Song not found", "This song is not defined", false).await;
            return Ok(());
        }
    }

    Ok(())
}
