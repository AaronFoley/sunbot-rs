use crate::{constants::SUCCESS_COLOUR, utils::send_err_msg, Context, Error, LavalinkUserData};
use futures::{future, StreamExt};
use humantime::format_duration;
use lavalink_rs::model::player;
use lavalink_rs::prelude::*;
use poise::serenity_prelude as serenity;
use std::ops::Deref;
use std::time::Duration;

async fn audio_request<T>(
    stage: &'static str,
    request: impl std::future::Future<Output = Result<T, lavalink_rs::error::LavalinkError>>,
) -> Result<T, Error> {
    tracing::info!(stage, "Starting audio request");
    match tokio::time::timeout(Duration::from_secs(45), request).await {
        Ok(result) => {
            tracing::info!(stage, success = result.is_ok(), "Audio request completed");
            Ok(result?)
        }
        Err(_) => {
            tracing::warn!(stage, "Audio request timed out");
            Err(Error::AudioTimeout(stage))
        }
    }
}

pub async fn join_channel(
    ctx: &Context<'_>,
    guild_id: serenity::GuildId,
    channel_id: Option<serenity::ChannelId>,
) -> Result<bool, Error> {
    let lock = crate::handlers::punish::audio_lock(guild_id).await;
    let _guard = tokio::time::timeout(Duration::from_secs(30), lock.lock())
        .await
        .map_err(|_| Error::AudioTimeout("waiting for another voice operation"))?;
    let lava_client = ctx
        .data()
        .lavalink
        .as_ref()
        .ok_or_else(|| Error::LavaClientNotAvailable)?;
    let manager = songbird::get(ctx.serenity_context()).await.unwrap().clone();

    if lava_client.get_player_context(guild_id).is_none() {
        let connect_to = match channel_id {
            Some(x) => x,
            None => {
                let guild = ctx.guild().unwrap().deref().clone();
                let user_channel_id = guild
                    .voice_states
                    .get(&ctx.author().id)
                    .and_then(|voice_state| voice_state.channel_id);

                match user_channel_id {
                    Some(channel) => channel,
                    None => {
                        send_err_msg(
                            *ctx,
                            "Error",
                            "You are not in a voice channel, please join one first.",
                            false,
                        )
                        .await;
                        return Ok(false);
                    }
                }
            }
        };

        tracing::info!(%guild_id, %connect_to, "Waiting for Discord voice handshake");
        let handler = match tokio::time::timeout(
            Duration::from_secs(30),
            manager.join_gateway(guild_id, connect_to),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!(%guild_id, "Discord voice handshake timed out");
                // Cancel the incomplete call so the next attempt starts fresh.
                let _ =
                    tokio::time::timeout(Duration::from_secs(5), manager.remove(guild_id)).await;
                return Err(Error::AudioTimeout("connecting to Discord voice"));
            }
        };

        match handler {
            Ok((connection_info, _)) => {
                tracing::info!(%guild_id, "Discord voice handshake completed");
                audio_request(
                    "creating the Lavalink player",
                    lava_client.create_player_context_with_data::<LavalinkUserData>(
                        guild_id,
                        player::ConnectionInfo {
                            endpoint: connection_info.endpoint,
                            session_id: connection_info.session_id,
                            token: connection_info.token,
                            channel_id: Some(connect_to.into()),
                        },
                        std::sync::Arc::new(LavalinkUserData {
                            voice_channel: connect_to,
                            cache: ctx.serenity_context().cache.clone(),
                            playback_lock: tokio::sync::Mutex::new(()),
                            channel_id: Some(ctx.channel_id()),
                            http: ctx.serenity_context().http.clone(),
                        }),
                    ),
                )
                .await?;
                return Ok(true);
            }
            Err(why) => {
                send_err_msg(
                    *ctx,
                    "Error",
                    format!("Error joining the channel: {}", why).as_str(),
                    false,
                )
                .await;
                return Err(why.into());
            }
        }
    }

    Ok(false)
}

/// Play a song in the voice channel you are connected in.
#[poise::command(slash_command, guild_only)]
pub async fn play(
    ctx: Context<'_>,
    #[description = "Search term or URL"]
    #[rest]
    term: String,
) -> Result<(), Error> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().ok_or_else(|| Error::GuildExpected)?;
    let lava_client = ctx
        .data()
        .lavalink
        .as_ref()
        .ok_or_else(|| Error::LavaClientNotAvailable)?;

    tracing::info!(%guild_id, "Preparing voice connection for playback");
    join_channel(&ctx, guild_id, None).await?;
    tracing::info!(%guild_id, "Voice connection prepared for playback");
    let Some(player) = lava_client.get_player_context(guild_id) else {
        return Ok(());
    };

    let query = if term.starts_with("http") {
        term
    } else {
        SearchEngines::YouTube.to_query(&term)?
    };

    let loaded_tracks =
        audio_request("loading tracks", lava_client.load_tracks(guild_id, &query)).await?;
    let mut playlist_info = None;
    let mut tracks: Vec<TrackInQueue> = match loaded_tracks.data {
        Some(TrackLoadData::Track(x)) => vec![x.into()],
        Some(TrackLoadData::Search(x)) => {
            first_search_track(x).into_iter().map(Into::into).collect()
        }
        Some(TrackLoadData::Playlist(x)) => {
            playlist_info = Some(x.info);
            x.tracks.iter().map(|x| x.clone().into()).collect()
        }
        Some(TrackLoadData::Error(x)) => {
            send_err_msg(ctx, "Error", x.message.as_str(), false).await;
            return Ok(());
        }
        _ => {
            ctx.say(format!("{:?}", loaded_tracks)).await?;
            return Ok(());
        }
    };

    let Some(first_track) = tracks.first() else {
        send_err_msg(
            ctx,
            "No tracks found",
            "Try a different search or URL.",
            true,
        )
        .await;
        return Ok(());
    };
    let track = first_track.track.clone();
    let track_count = tracks.len();
    let queue = player.get_queue();
    let mut duration = 0;
    let position = queue.get_count().await.unwrap_or(0) + 1;

    for i in &mut tracks {
        i.track.user_data = Some(serde_json::json!({"requester_id": ctx.author().id.get()}));
        duration += i.track.info.length;
    }

    let playing = audio_request("checking the player", player.get_player())
        .await?
        .track
        .is_some();
    let (start, pending) = split_playback(tracks, playing);
    if let Some(start) = start {
        audio_request("starting playback", player.play(&start.track)).await?;
    }
    queue.append(pending.into())?;

    // A single track starts immediately rather than entering the pending queue.
    if queue.get_count().await.unwrap_or(0) == 0 {
        ctx.send(
            poise::CreateReply::default()
                .content(format!("Started playing: {}", track.info.title))
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let mut embed = serenity::CreateEmbed::default().color(SUCCESS_COLOUR);

    embed = if let Some(info) = playlist_info {
        embed
            .author(
                serenity::CreateEmbedAuthor::new("Playlist added to queue")
                    .icon_url(ctx.author().avatar_url().unwrap_or_default()),
            )
            .description(format!("Added playlist {}", info.name))
            .field("Tracks", track_count.to_string(), false)
            .field(
                "Position",
                format!("#{}-{}", position, position + track_count - 1),
                true,
            )
            .field(
                "Duration",
                format_duration(Duration::from_millis(duration)).to_string(),
                true,
            )
    } else {
        embed
            .author(
                serenity::CreateEmbedAuthor::new("Added to queue")
                    .icon_url(ctx.author().avatar_url().unwrap_or_default()),
            )
            .image(track.info.artwork_url.as_ref().unwrap_or(&String::new()))
            .description(format!(
                "[{}](<{}>)",
                track.info.title,
                track.info.uri.as_ref().unwrap_or(&String::new())
            ))
            .field("Position", format!("#{}", position), true)
            .field(
                "Duration",
                format_duration(Duration::from_millis(duration)).to_string(),
                true,
            )
    };

    ctx.send(poise::CreateReply::default().embed(embed)).await?;

    Ok(())
}

/// Join the specified voice channel or the one you are currently in.
#[poise::command(slash_command, guild_only)]
pub async fn join(
    ctx: Context<'_>,
    #[description = "The channel ID to join to."]
    #[channel_types("Voice")]
    channel_id: Option<serenity::ChannelId>,
) -> Result<(), Error> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().ok_or(Error::GuildExpected)?;
    if !join_channel(&ctx, guild_id, channel_id).await? {
        return Ok(());
    }

    ctx.send(
        poise::CreateReply::default()
            .content("Joined the voice channel.")
            .ephemeral(true),
    )
    .await?;

    Ok(())
}

/// Stop Playing music and Leave the current voice channel.
#[poise::command(slash_command, guild_only)]
pub async fn leave(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().ok_or_else(|| Error::GuildExpected)?;
    let lock = crate::handlers::punish::audio_lock(guild_id).await;
    let _guard = lock.lock().await;
    let manager = songbird::get(ctx.serenity_context()).await.unwrap().clone();
    let lava_client = ctx
        .data()
        .lavalink
        .as_ref()
        .ok_or_else(|| Error::LavaClientNotAvailable)?;

    if lava_client.get_player_context(guild_id).is_none() {
        send_err_msg(ctx, "Error", "Im not playing anything! :rage:", false).await;
        return Ok(());
    }

    let _ = lava_client.delete_player(guild_id).await;

    if manager.get(guild_id).is_some() {
        manager.remove(guild_id).await?;
    }

    let embed = serenity::CreateEmbed::new()
        .author(
            serenity::CreateEmbedAuthor::new("Stopped playing music")
                .icon_url(ctx.author().avatar_url().unwrap_or_default()),
        )
        .color(SUCCESS_COLOUR);

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}

/// Pauses playing music
#[poise::command(slash_command, guild_only)]
pub async fn pause(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or_else(|| Error::GuildExpected)?;
    let lava_client = ctx
        .data()
        .lavalink
        .as_ref()
        .ok_or_else(|| Error::LavaClientNotAvailable)?;

    let Some(player) = lava_client.get_player_context(guild_id) else {
        send_err_msg(
            ctx,
            "Error",
            "Join the bot to a voice channel first.",
            false,
        )
        .await;
        return Ok(());
    };
    player.set_pause(true).await?;

    let embed = serenity::CreateEmbed::new()
        .author(
            serenity::CreateEmbedAuthor::new("Paused Music")
                .icon_url(ctx.author().avatar_url().unwrap_or_default()),
        )
        .color(SUCCESS_COLOUR);
    ctx.send(poise::CreateReply::default().embed(embed)).await?;

    Ok(())
}

/// Resumes playing music
#[poise::command(slash_command, guild_only)]
pub async fn resume(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or_else(|| Error::GuildExpected)?;
    let lava_client = ctx
        .data()
        .lavalink
        .as_ref()
        .ok_or_else(|| Error::LavaClientNotAvailable)?;

    let Some(player) = lava_client.get_player_context(guild_id) else {
        send_err_msg(
            ctx,
            "Error",
            "Join the bot to a voice channel first.",
            false,
        )
        .await;
        return Ok(());
    };

    player.set_pause(false).await?;

    let embed = serenity::CreateEmbed::new()
        .author(
            serenity::CreateEmbedAuthor::new("Resumed Music")
                .icon_url(ctx.author().avatar_url().unwrap_or_default()),
        )
        .color(SUCCESS_COLOUR);
    ctx.send(poise::CreateReply::default().embed(embed)).await?;

    Ok(())
}

/// Skip the current song
#[poise::command(slash_command, guild_only)]
pub async fn skip(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or(Error::GuildExpected)?;

    let lava_client = match &ctx.data().lavalink {
        Some(x) => x,
        None => {
            send_err_msg(ctx, "Error", "Lavalink client is not available.", false).await;
            return Ok(());
        }
    };

    let Some(player) = lava_client.get_player_context(guild_id) else {
        send_err_msg(
            ctx,
            "Error",
            "Join the bot to a voice channel first.",
            false,
        )
        .await;
        return Ok(());
    };

    player.skip()?;

    // If queue is empty and nothing is playing, send a different message
    if player.get_queue().get_count().await? == 0 && player.get_player().await?.track.is_none() {
        let embed = serenity::CreateEmbed::new()
            .author(
                serenity::CreateEmbedAuthor::new("Skipped")
                    .icon_url(ctx.author().avatar_url().unwrap_or_default()),
            )
            .color(SUCCESS_COLOUR)
            .description("Queue is empty");
        ctx.send(poise::CreateReply::default().embed(embed)).await?;
    } else {
        let embed = serenity::CreateEmbed::new()
            .author(
                serenity::CreateEmbedAuthor::new("Skipped")
                    .icon_url(ctx.author().avatar_url().unwrap_or_default()),
            )
            .color(SUCCESS_COLOUR)
            .description("Skipped the current song");
        ctx.send(poise::CreateReply::default().embed(embed)).await?;
    }

    Ok(())
}

/// Displays the current queue
#[poise::command(slash_command, guild_only)]
pub async fn queue(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or_else(|| Error::GuildExpected)?;
    let lava_client = ctx
        .data()
        .lavalink
        .as_ref()
        .ok_or_else(|| Error::LavaClientNotAvailable)?;

    let Some(player) = lava_client.get_player_context(guild_id) else {
        send_err_msg(
            ctx,
            "Error",
            "Join the bot to a voice channel first.",
            false,
        )
        .await;
        return Ok(());
    };

    let queue = player.get_queue();
    let queue_count = queue.get_count().await?;
    let player_data = player.get_player().await?;
    let max = queue_count.min(5);
    let mut queue_message = queue
        .enumerate()
        .take_while(|(idx, _)| future::ready(*idx < max))
        .map(|(idx, x)| {
            format!(
                "**{} - **[{} - {}](<{}>)\n*Requested By {}* | {}\n",
                idx + 1,
                x.track.info.author,
                x.track.info.title,
                x.track.info.uri.as_ref().unwrap_or(&String::new()),
                requester_label(x.track.user_data.as_ref()),
                format_duration(Duration::from_millis(x.track.info.length)),
            )
        })
        .collect::<Vec<_>>()
        .await
        .join("\n");

    if queue_count > max {
        queue_message.push_str(&format!("\n\nAnd {} more...", queue_count - max));
    }

    let song_position = player.get_player().await?.state.position;
    let now_playing_message = if let Some(track) = player_data.track {
        format!(
            "[{} - {}](<{}>)\n*Requested by {}*\n{} Left\n",
            track.info.author,
            track.info.title,
            track.info.uri.as_ref().unwrap_or(&String::new()),
            requester_label(track.user_data.as_ref()),
            format_duration(Duration::from_millis(
                track.info.length.saturating_sub(song_position) / 1000 * 1000
            ))
        )
    } else {
        "No song is currently playing".to_string()
    };

    let embed = serenity::CreateEmbed::new()
        .title("Queue")
        .color(SUCCESS_COLOUR)
        .field("Now Playing", now_playing_message, false)
        .field("Queue", queue_message, false);

    ctx.send(poise::CreateReply::default().embed(embed)).await?;

    Ok(())
}

fn requester_label(data: Option<&serde_json::Value>) -> String {
    data.and_then(|value| value.get("requester_id"))
        .and_then(serde_json::Value::as_u64)
        .map(|id| format!("<@!{id}>"))
        .unwrap_or_else(|| "Automatic playback".to_owned())
}

pub(crate) fn first_search_track(
    tracks: Vec<lavalink_rs::model::track::TrackData>,
) -> Option<lavalink_rs::model::track::TrackData> {
    tracks.into_iter().next()
}

fn split_playback<T>(tracks: Vec<T>, playing: bool) -> (Option<T>, Vec<T>) {
    let mut tracks = tracks.into_iter();
    let start = if playing { None } else { tracks.next() };
    (start, tracks.collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_search_is_a_normal_no_result() {
        assert!(first_search_track(vec![]).is_none());
    }
    #[test]
    fn idle_single_track_starts_without_a_second_removal() {
        assert_eq!(split_playback(vec!["song"], false), (Some("song"), vec![]));
        assert_eq!(split_playback::<&str>(vec![], false), (None, vec![]));
    }
    #[test]
    fn playlists_keep_order_and_playing_tracks_are_not_replaced() {
        assert_eq!(split_playback(vec![1, 2, 3], false), (Some(1), vec![2, 3]));
        assert_eq!(split_playback(vec![1, 2, 3], true), (None, vec![1, 2, 3]));
    }
    #[test]
    fn automatic_tracks_have_a_readable_requester() {
        assert_eq!(requester_label(None), "Automatic playback");
        assert_eq!(
            requester_label(Some(&serde_json::json!({"requester_id": 42}))),
            "<@!42>"
        );
    }
}
