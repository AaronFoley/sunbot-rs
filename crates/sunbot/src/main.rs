use crate::error::Error;
use lavalink_rs::{model::events, prelude::*};
use poise::serenity_prelude as serenity;
use sea_orm::DatabaseConnection;
use songbird::SerenityInit;
use sunbot_config::{self, config::SunbotConfig};
use sunbot_db::{get_db, init_db};
use tracing::{info, warn, Level};
use tracing_subscriber::{filter, prelude::*};

mod commands;
mod constants;
mod error;
mod handlers;
mod utils;

pub mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

pub struct Data {
    config: &'static SunbotConfig,
    openai_client: Option<async_openai::Client<async_openai::config::OpenAIConfig>>,
    lavalink: Option<LavalinkClient>,
    db: &'static DatabaseConnection,
}

pub struct LavalinkUserData {
    voice_channel: serenity::ChannelId,
    cache: std::sync::Arc<serenity::Cache>,
    playback_lock: tokio::sync::Mutex<()>,
    channel_id: Option<serenity::ChannelId>,
    http: std::sync::Arc<serenity::Http>,
}

type Context<'a> = poise::Context<'a, Data, Error>;

async fn on_ready(
    ctx: &serenity::Context,
    ready: &serenity::Ready,
    _framework: &poise::Framework<Data, Error>,
) -> Result<Data, Error> {
    info!("Logged in as {}", ready.user.name);
    let config: &SunbotConfig = sunbot_config::get_config();

    // Initialize the database
    init_db(&config.database.url).await;

    // Configure OpenAI
    let openai_client = if !config.openai.api_key.is_empty() {
        Some(async_openai::Client::with_config(
            async_openai::config::OpenAIConfig::new().with_api_key(config.openai.api_key.as_str()),
        ))
    } else {
        None
    };

    // Setup Lavalink
    let lavalink_client = if !config.lavalink.host.is_empty() {
        let events = events::Events {
            raw: Some(handlers::lavalink::raw_event),
            ready: Some(handlers::lavalink::ready_event),
            track_start: Some(handlers::lavalink::track_start),
            track_end: Some(handlers::lavalink::track_end),
            ..Default::default()
        };

        let lavalink_host = format!("{}:{}", config.lavalink.host, config.lavalink.port);

        let node_local = NodeBuilder {
            hostname: lavalink_host,
            is_ssl: config.lavalink.use_ssl,
            events: events::Events::default(),
            password: config.lavalink.password.clone(),
            user_id: ctx.cache.current_user().id.into(),
            session_id: None,
        };

        Some(
            LavalinkClient::new(
                events,
                vec![node_local],
                NodeDistributionStrategy::round_robin(),
            )
            .await,
        )
    } else {
        None
    };

    Ok(Data {
        config,
        openai_client,
        lavalink: lavalink_client,
        db: get_db().await,
    })
}

async fn bot_entrypoint() {
    let config = sunbot_config::get_config();

    let commands = vec![
        commands::register::register_commands(),
        commands::meta::ping(),
        commands::meta::about(),
        commands::openai::askgpt(),
        commands::openai::genimage(),
        commands::music::join(),
        commands::music::leave(),
        commands::music::play(),
        commands::music::pause(),
        commands::music::resume(),
        commands::music::skip(),
        commands::music::queue(),
        commands::punish::punish(),
    ];

    let options = poise::FrameworkOptions {
        commands,
        prefix_options: poise::PrefixFrameworkOptions {
            prefix: Some("~".into()),
            execute_self_messages: false,
            execute_untracked_edits: true,
            mention_as_prefix: false,
            ..Default::default()
        },
        event_handler: |framework, event| {
            Box::pin(handlers::handler(
                framework.serenity_context,
                event,
                framework,
            ))
        },
        on_error: |error| Box::pin(error::on_error(error)),
        initialize_owners: true,
        ..Default::default()
    };

    let framework = poise::Framework::builder()
        .setup(|ctx, ready, framework| Box::pin(on_ready(ctx, ready, framework)))
        .options(options)
        .build();

    let intents = serenity::GatewayIntents::all();

    let client = serenity::ClientBuilder::new(config.discord.token.as_str(), intents)
        .register_songbird()
        .framework(framework)
        .await;

    client.unwrap().start().await.unwrap()
}

fn main() {
    let config = sunbot_config::load_config();

    if config.discord.token.is_empty() {
        panic!("Discord token is not set in the configuration file");
    }

    if config.sentry.dsn.is_empty() {
        warn!("Sentry initialized with empty DSN - will be disabled")
    }

    let mut sentry_options = sentry::ClientOptions::default();
    sentry_options.release = sentry::release_name!();
    let _guard = sentry::init((config.sentry.dsn.as_str(), sentry_options));

    // Configure logging with Sentry
    let stdout_log = tracing_subscriber::fmt::layer()
        .compact()
        .with_filter(filter::LevelFilter::from(Level::INFO));

    tracing_subscriber::registry()
        .with(stdout_log)
        .with(sentry_tracing::layer())
        .init();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { bot_entrypoint().await });
}
