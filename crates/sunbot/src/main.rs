use lavalink_rs::{model::events, prelude::*};
use poise::serenity_prelude as serenity;
use songbird::SerenityInit;
use sunbot_config::{self, config::SunbotConfig};
use tracing::{info, warn, Level};
use tracing_subscriber::FmtSubscriber;

mod commands;
mod handlers;
mod utils;

pub mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

pub struct Data {
    config: &'static SunbotConfig,
    openai_client: Option<async_openai::Client<async_openai::config::OpenAIConfig>>,
    lavalink: LavalinkClient,
}

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

fn load_config() -> &'static SunbotConfig {
    let cfg_path = std::env::var("SUNBOT_CONFIG_FILE").unwrap_or(String::from("config.toml"));
    info!("Loading configuration from: {}", cfg_path);
    sunbot_config::load_config(&cfg_path);
    sunbot_config::get_config()
}

async fn on_ready(
    ctx: &serenity::Context,
    ready: &serenity::Ready,
    _framework: &poise::Framework<Data, Error>,
) -> Result<Data, Error> {
    info!("Logged in as {}", ready.user.name);
    let config: &SunbotConfig = sunbot_config::get_config();

    // TODO: Setup/Configure DB access

    // Attempt to get the openai client
    let openai_client = Some(async_openai::Client::with_config(
        async_openai::config::OpenAIConfig::new().with_api_key(config.openai.api_key.as_str()),
    ));

    // Setup Lavalink
    let events = events::Events {
        raw: Some(handlers::lavalink::raw_event),
        ready: Some(handlers::lavalink::ready_event),
        track_start: Some(handlers::lavalink::track_start),
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

    let lavalink_client = LavalinkClient::new(
        events,
        vec![node_local],
        NodeDistributionStrategy::round_robin(),
    )
    .await;

    Ok(Data {
        config: config,
        openai_client: openai_client,
        lavalink: lavalink_client,
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
    ];

    let options = poise::FrameworkOptions {
        commands: commands,
        prefix_options: poise::PrefixFrameworkOptions {
            prefix: Some("~".into()),
            execute_self_messages: false,
            execute_untracked_edits: true,
            mention_as_prefix: true,
            ..Default::default()
        },
        event_handler: |ctx, event, framework, data| {
            Box::pin(handlers::handler(ctx, event, framework, data))
        },
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
    // Configue Logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let config = load_config();

    if config.discord.token.is_empty() {
        panic!("Discord token is not set in the configuration file");
    }

    if config.sentry.dsn.is_empty() {
        warn!("Sentry initialized with empty DSN - will be disabled")
    }

    let _guard = sentry::init((
        config.sentry.dsn.as_str(),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            ..Default::default()
        },
    ));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { bot_entrypoint().await });
}
