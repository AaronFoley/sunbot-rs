use crate::{Data, Error, LavalinkUserData};
use lavalink_rs::{model::player, prelude::*};
use poise::serenity_prelude as serenity;
use sea_orm::*;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};
use sunbot_db::entities::{prelude::*, punishment, punishment_channel, punishment_song};
use tokio::sync::Mutex;
use tracing::{info, warn};
static AUDIO_LOCKS: LazyLock<Mutex<HashMap<serenity::GuildId, Arc<Mutex<()>>>>> =
    LazyLock::new(Mutex::default);
pub(crate) async fn audio_lock(guild: serenity::GuildId) -> Arc<Mutex<()>> {
    AUDIO_LOCKS.lock().await.entry(guild).or_default().clone()
}

#[derive(Debug, PartialEq)]
enum PlaybackAction {
    Keep,
    Refill,
    Disconnect,
}
fn playback_action(
    punishment: bool,
    occupied: Option<bool>,
    queued: usize,
    playing: bool,
) -> PlaybackAction {
    match (punishment, occupied, queued, playing) {
        (true, Some(false), _, _) => PlaybackAction::Disconnect,
        (true, Some(true), 0, false) => PlaybackAction::Refill,
        _ => PlaybackAction::Keep,
    }
}

/// Refill only an idle punishment player. Bad or unavailable songs are skipped.
pub async fn refill_songs(
    client: &LavalinkClient,
    guild_id: serenity::GuildId,
) -> Result<(), Error> {
    let Some(player) = client.get_player_context(guild_id) else {
        return Ok(());
    };
    let Ok(data) = player.data::<LavalinkUserData>() else {
        return Ok(());
    };
    let _guard = data.playback_lock.lock().await;
    if playback_action(
        data.channel_id.is_none(),
        channel_occupied(&data, guild_id),
        player.get_queue().get_count().await?,
        player.get_player().await?.track.is_some(),
    ) != PlaybackAction::Refill
    {
        return Ok(());
    }
    let songs = PunishmentSong::find()
        .filter(punishment_song::Column::Guild.eq(guild_id.get() as i64))
        .order_by_asc(punishment_song::Column::Id)
        .all(sunbot_db::get_db().await)
        .await?;
    let mut tracks = Vec::new();
    for song in songs {
        let loaded = match client.load_tracks(guild_id, &song.url).await {
            Ok(loaded) => loaded,
            Err(error) => {
                warn!(%error, url = %song.url, "Cannot load punishment song");
                continue;
            }
        };
        let track = match loaded.data {
            Some(TrackLoadData::Track(track)) => Some(track),
            Some(TrackLoadData::Search(tracks)) => {
                crate::commands::music::first_search_track(tracks)
            }
            _ => None,
        };
        if let Some(track) = track {
            tracks.push(TrackInQueue::from(track));
        } else {
            warn!(url = %song.url, "Punishment song unavailable");
        }
    }
    // The player may have been removed while tracks were loading.
    if client.get_player_context(guild_id).is_none() {
        return Ok(());
    }
    if channel_occupied(&data, guild_id).unwrap_or(false) && !tracks.is_empty() {
        player.get_queue().append(tracks.into())?;
        player.skip()?;
    }
    Ok(())
}

pub async fn on_voice_state_change(
    ctx: &serenity::Context,
    framework: poise::FrameworkContext<'_, Data, Error>,
    old: &Option<serenity::VoiceState>,
    new: &serenity::VoiceState,
) -> Result<(), Error> {
    if new.user_id == framework.bot_id() {
        return Ok(());
    }
    let Some(guild_id) = new.guild_id else {
        return Ok(());
    };
    let lock = audio_lock(guild_id).await;
    let _guard = lock.lock().await;
    let Some(pchannel) = PunishmentChannel::find()
        .filter(punishment_channel::Column::Guild.eq(guild_id.get() as i64))
        .one(framework.user_data.db)
        .await?
    else {
        return Ok(());
    };
    let punishment_channel = serenity::ChannelId::new(pchannel.channel as u64);

    if new.channel_id == Some(punishment_channel) {
        // Punishment enforcement works even when audio is disabled.
        let Some(client) = framework.user_data.lavalink.as_ref() else {
            return Ok(());
        };
        start_punishment(ctx, client, guild_id, punishment_channel).await?;
        return Ok(());
    }

    if new.channel_id.is_some()
        && Punishment::find()
            .filter(punishment::Column::Guild.eq(guild_id.get() as i64))
            .filter(punishment::Column::User.eq(new.user_id.get() as i64))
            .filter(punishment::Column::ExpiresAt.gt(chrono::Utc::now().naive_utc()))
            .one(framework.user_data.db)
            .await?
            .is_some()
    {
        guild_id
            .edit_member(
                &ctx.http,
                new.user_id,
                serenity::EditMember::new().voice_channel(punishment_channel),
            )
            .await?;
        return Ok(());
    }

    if old.as_ref().and_then(|state| state.channel_id).is_some() {
        if let Some(client) = framework.user_data.lavalink.as_ref() {
            disconnect_punishment(ctx, client, guild_id, true).await?;
        }
    }
    Ok(())
}

fn channel_occupied(data: &LavalinkUserData, guild: serenity::GuildId) -> Option<bool> {
    let bot = data.cache.current_user().id;
    data.cache.guild(guild).map(|guild| {
        guild
            .voice_states
            .values()
            .any(|state| state.user_id != bot && state.channel_id == Some(data.voice_channel))
    })
}

/// Keep ordinary music players untouched; recheck occupancy after waiting for refill.
pub(crate) async fn disconnect_punishment(
    ctx: &serenity::Context,
    client: &LavalinkClient,
    guild: serenity::GuildId,
    only_empty: bool,
) -> Result<(), Error> {
    let Some(player) = client.get_player_context(guild) else {
        return Ok(());
    };
    let Ok(data) = player.data::<LavalinkUserData>() else {
        return Ok(());
    };
    let _guard = data.playback_lock.lock().await;
    let action = playback_action(
        data.channel_id.is_none(),
        channel_occupied(&data, guild),
        0,
        false,
    );
    if data.channel_id.is_some() || (only_empty && action != PlaybackAction::Disconnect) {
        return Ok(());
    }
    let deletion = client.delete_player(guild).await;
    if let Some(manager) = songbird::get(ctx).await {
        manager.remove(guild).await?;
    }
    deletion?;
    Ok(())
}

pub(crate) async fn start_punishment(
    ctx: &serenity::Context,
    client: &LavalinkClient,
    guild_id: serenity::GuildId,
    punishment_channel: serenity::ChannelId,
) -> Result<(), Error> {
    let occupied = ctx.cache.guild(guild_id).is_some_and(|guild| {
        guild.voice_states.values().any(|state| {
            state.user_id != ctx.cache.current_user().id
                && state.channel_id == Some(punishment_channel)
        })
    });
    if !occupied {
        return Ok(());
    }
    if client.get_player_context(guild_id).is_none() {
        let manager = songbird::get(ctx).await.unwrap();
        let (connection, _) = manager.join_gateway(guild_id, punishment_channel).await?;
        info!(%guild_id, "Joining punishment channel");
        client
            .create_player_context_with_data(
                guild_id,
                player::ConnectionInfo {
                    endpoint: connection.endpoint,
                    session_id: connection.session_id,
                    token: connection.token,
                    channel_id: Some(punishment_channel.into()),
                },
                std::sync::Arc::new(LavalinkUserData {
                    voice_channel: punishment_channel,
                    cache: ctx.cache.clone(),
                    playback_lock: tokio::sync::Mutex::new(()),
                    channel_id: None,
                    http: ctx.http.clone(),
                }),
            )
            .await?;
    }
    refill_songs(client, guild_id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeat_only_after_the_current_track_and_queue_are_finished() {
        assert_eq!(
            playback_action(true, Some(true), 0, false),
            PlaybackAction::Refill
        );
        assert_eq!(
            playback_action(true, Some(true), 1, false),
            PlaybackAction::Keep
        );
        assert_eq!(
            playback_action(true, Some(true), 0, true),
            PlaybackAction::Keep
        );
        assert_eq!(
            playback_action(false, Some(true), 0, false),
            PlaybackAction::Keep
        );
    }
    #[test]
    fn last_participant_leaving_disconnects_but_a_new_arrival_cancels_cleanup() {
        assert_eq!(
            playback_action(true, Some(false), 3, true),
            PlaybackAction::Disconnect
        );
        assert_eq!(
            playback_action(true, Some(true), 3, true),
            PlaybackAction::Keep
        );
        assert_eq!(playback_action(true, None, 0, false), PlaybackAction::Keep);
        assert_eq!(
            playback_action(false, Some(false), 0, false),
            PlaybackAction::Keep
        );
    }
    #[tokio::test]
    async fn channel_changes_and_voice_events_share_a_guild_lock() {
        let guild = serenity::GuildId::new(999);
        let a = audio_lock(guild).await;
        let b = audio_lock(guild).await;
        let _guard = a.lock().await;
        assert!(b.try_lock().is_err());
        assert!(audio_lock(serenity::GuildId::new(1000))
            .await
            .try_lock()
            .is_ok());
    }
}
