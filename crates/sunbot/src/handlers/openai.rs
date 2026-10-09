use crate::{utils::is_reply_or_mention, Data, Error};
use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessageContentPartImageArgs,
    ChatCompletionRequestMessageContentPartTextArgs, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContentPart,
    CreateChatCompletionRequestArgs,
};
use poise::serenity_prelude as serenity;
use rand::RngExt;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::Instant,
};
use tokio::sync::Mutex;
use tracing::{error, info};

#[derive(Default)]
struct Cooldown {
    last_success: Option<Instant>,
}
impl Cooldown {
    fn ready(&self, now: Instant, seconds: u64) -> bool {
        self.last_success
            .is_none_or(|last| now.saturating_duration_since(last).as_secs() >= seconds)
    }
    fn finish(&mut self, successful: bool, now: Instant) {
        if successful {
            self.last_success = Some(now);
        }
    }
}
static COOLDOWNS: LazyLock<Mutex<HashMap<u64, Arc<Mutex<Cooldown>>>>> =
    LazyLock::new(Mutex::default);
async fn cooldown_for(key: u64) -> Arc<Mutex<Cooldown>> {
    COOLDOWNS.lock().await.entry(key).or_default().clone()
}

// Generate a response to a message
pub async fn generate_response(
    ctx: &serenity::Context,
    framework: poise::FrameworkContext<'_, Data, Error>,
    message: &serenity::Message,
) -> Result<(), Error> {
    if framework.user_data.openai_client.is_none() {
        return Ok(());
    }
    // Gather some context
    let mut messages = ctx
        .http
        .get_messages(
            message.channel_id,
            Some(serenity::MessagePagination::Before(message.id)),
            Some(framework.user_data.config.openai.auto.max_messages),
        )
        .await?;

    messages.insert(0, message.clone());

    let mut chat_messages: Vec<async_openai::types::chat::ChatCompletionRequestMessage> = vec![];

    // Include system context
    for ctx in framework.user_data.config.openai.auto.system_context.iter() {
        chat_messages.push(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(ctx.as_str())
                .build()?
                .into(),
        );
    }

    for msg in messages.iter().rev() {
        // If this message is too old ignore it
        let diff = message.timestamp.timestamp() - msg.timestamp.timestamp();
        if diff > framework.user_data.config.openai.auto.max_message_age {
            continue;
        }

        // If this is sent by us use ChatCompletionRequestAssistantMessage
        if msg.author.id == framework.bot_id() {
            chat_messages.push(
                ChatCompletionRequestAssistantMessageArgs::default()
                    .content(msg.content.as_str())
                    .name(msg.author.name.as_str())
                    .build()?
                    .into(),
            );
            continue;
        }
        // Otherwise ignore messages from other bots
        else if msg.author.bot {
            continue;
        }

        // Otherwise, this is a user message
        let mut user_content: Vec<ChatCompletionRequestUserMessageContentPart> =
            vec![ChatCompletionRequestMessageContentPartTextArgs::default()
                .text(msg.content.as_str())
                .build()?
                .into()];

        // If we have use_vision enabled
        if framework.user_data.config.openai.auto.use_vision {
            for attachment in msg.attachments.iter() {
                if let Some(content_type) = attachment.content_type.as_deref() {
                    if content_type.to_lowercase().starts_with("image") {
                        info!("Found image attachment: {}", attachment.url.as_str());
                        user_content.push(
                            ChatCompletionRequestMessageContentPartImageArgs::default()
                                .image_url(attachment.url.as_str())
                                .build()?
                                .into(),
                        );
                    }
                }
            }
        }

        // OpenAI is very strict about the name, we need to make sure it matches ^[a-zA-Z0-9_-]+$
        // Remove any special characters, and replace spaces with underscores
        let username = msg
            .author
            .name
            .replace(' ', "_")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>();

        // Not sure what is causing this, log the changes so we might know more
        if username != msg.author.name {
            info!("Changed username from {} to {}", msg.author.name, username);
        }

        chat_messages.push(
            ChatCompletionRequestUserMessageArgs::default()
                .content(user_content)
                .name(format!("{}__{}", username, msg.author.id))
                .build()?
                .into(),
        );
    }

    let openai_tasks = async {
        let client = framework.user_data.openai_client.as_ref().unwrap();

        let mut request = CreateChatCompletionRequestArgs::default()
            .model(framework.user_data.config.openai.auto.model.as_str())
            .messages(chat_messages.clone())
            .max_completion_tokens(framework.user_data.config.openai.auto.max_tokens)
            .build()?;

        crate::commands::openai::configure_reasoning(&mut request);
        let resp = client.chat().create(request).await?;

        // Send the response
        message
            .reply(ctx, crate::commands::openai::response_text(&resp)?)
            .await?;
        Ok::<(), Error>(())
    };

    let result = openai_tasks.await;
    if let Err(e) = &result {
        error!("Error generating response: {:?}", e);
        info!("Request: {:?}", chat_messages);
    }

    result
}

// Handle replies to the Bot
pub async fn handle_reply(
    ctx: &serenity::Context,
    framework: poise::FrameworkContext<'_, Data, Error>,
    message: &serenity::Message,
) -> Result<(), Error> {
    if framework.user_data.openai_client.is_none()
        || message.author.bot
        || message.content.is_empty()
    {
        return Ok(());
    }

    if is_reply_or_mention(ctx, message, framework.bot_id()).await {
        info!("Triggered Reply on message: {}", message.content);
        return generate_response(ctx, framework, message).await;
    }

    Ok(())
}

pub async fn handle_random_message(
    ctx: &serenity::Context,
    framework: poise::FrameworkContext<'_, Data, Error>,
    message: &serenity::Message,
) -> Result<(), Error> {
    if framework.user_data.openai_client.is_none()
        || message.author.bot
        || message.content.is_empty()
    {
        return Ok(());
    }

    if is_reply_or_mention(ctx, message, framework.bot_id()).await {
        return Ok(());
    }

    // Message must be longer than min length
    if message.content.len() < framework.user_data.config.openai.auto.random.min_length as usize {
        return Ok(());
    }

    let key = message
        .guild_id
        .map_or(message.channel_id.get(), |guild| guild.get());
    let cooldown = cooldown_for(key).await;
    let mut cooldown = cooldown.lock().await;
    if !cooldown.ready(
        Instant::now(),
        framework.user_data.config.openai.auto.random.cooldown,
    ) {
        return Ok(());
    }
    if rand::rng().random::<f64>() < framework.user_data.config.openai.auto.random.trigger_chance {
        let result = generate_response(ctx, framework, message).await;
        cooldown.finish(result.is_ok(), Instant::now());
        return result;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn failed_attempts_do_not_consume_other_guilds_cooldowns() {
        let now = Instant::now();
        let a = cooldown_for(10001).await;
        let b = cooldown_for(10002).await;
        let mut a = a.lock().await;
        a.finish(false, now);
        assert!(a.ready(now, 600));
        a.finish(true, now);
        assert!(!a.ready(now, 600));
        assert!(b.lock().await.ready(now, 600));
        assert!(!a.ready(now + std::time::Duration::from_secs(599), 600));
        assert!(a.ready(now + std::time::Duration::from_secs(600), 600));
    }
    #[tokio::test]
    async fn simultaneous_requests_share_the_same_guild_lock() {
        let a = cooldown_for(10003).await;
        let b = cooldown_for(10003).await;
        let mut guard = a.lock().await;
        assert!(b.try_lock().is_err());
        guard.finish(true, Instant::now());
        drop(guard);
        assert!(!b.lock().await.ready(Instant::now(), 600));
    }
}
