use crate::{Context, Error};
use humantime::format_duration;
use serenity::builder::CreateEmbed;
use std::time::Duration;

/// Displays the ping/latency of the bot
#[poise::command(slash_command)]
pub async fn ping(ctx: Context<'_>) -> Result<(), Error> {
    let latency = ctx.ping().await;
    ctx.say(format!("Pong! ({:?})", latency)).await?;
    Ok(())
}

/// Displays information about this bot
#[poise::command(slash_command)]
pub async fn about(ctx: Context<'_>) -> Result<(), Error> {
    let user = ctx.framework().bot_id().to_user(&ctx.http()).await?;

    // Somehow get information about the process
    let pid = sysinfo::get_current_pid().expect("Unable to get current process ID");
    let s = sysinfo::System::new_all();
    let process = s.process(pid).expect("Unable to get process info");

    // Somehow get information about poise/serentiy
    let dependencies: Vec<String> = crate::built_info::DIRECT_DEPENDENCIES
        .iter()
        .map(|&(a, b)| format!("{}: {}", a, b))
        .collect();

    let embed = CreateEmbed::new()
        .title(format!("{}'s Information", user.name))
        .thumbnail(user.avatar_url().unwrap_or(user.default_avatar_url()))
        .field(
            "Language",
            format!("Rust {}", crate::built_info::RUSTC_VERSION),
            false,
        )
        .field("Bot Version", crate::built_info::PKG_VERSION, false)
        .field("Dependencies", dependencies.join("\n"), false)
        .field(
            "Uptime",
            format_duration(Duration::from_secs(process.run_time())).to_string(),
            false,
        )
        .field("CPU Usage", format!("{:.2}", process.cpu_usage()), false)
        .field(
            "Memory",
            format!("{:.2} MB", (process.memory() / 1024 / 1024)),
            false,
        )
        .field("Source", "https://github.com/AaronFoley/sunbot-rs/", false)
        .field("Author", "<@116586345115287558>", false);

    let reply = poise::CreateReply::default().embed(embed);

    ctx.send(reply).await?;

    Ok(())
}
