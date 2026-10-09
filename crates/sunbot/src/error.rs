use poise::builtins::on_error as poise_on_error;
use poise::FrameworkError;
use thiserror::Error;

use crate::Data;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Expected command to be run within a guild")]
    GuildExpected,
    #[error("Audio Streaming is not configured for this bot")]
    LavaClientNotAvailable,
    #[error("Audio timed out while {0}. Check the Lavalink logs and try again.")]
    AudioTimeout(&'static str),
    #[error("OpenAI is not configured for this bot")]
    OpenAIUnavailable,
    #[error(
        "Punishment duration must be a positive number of seconds within the supported date range"
    )]
    InvalidDuration,
    #[error(transparent)]
    Framework(Box<poise::serenity_prelude::Error>),
    #[error("The API returned no usable content")]
    EmptyResponse,
    #[error("Please attach an image and enable vision in the configuration")]
    InvalidImage,
    #[error("This image model only supports the square size")]
    InvalidImageSize,
    #[error("Image count must be between 1 and 10")]
    InvalidImageCount,
    #[error(transparent)]
    Base64(#[from] base64::DecodeError),
    #[error(transparent)]
    Lavalink(#[from] lavalink_rs::error::LavalinkError),
    #[error(transparent)]
    Songbird(#[from] songbird::error::JoinError),
    #[error(transparent)]
    OpenAI(#[from] async_openai::error::OpenAIError),
    #[error(transparent)]
    SeaORM(#[from] sea_orm::DbErr),
}

impl From<poise::serenity_prelude::Error> for Error {
    fn from(error: poise::serenity_prelude::Error) -> Self {
        Self::Framework(Box::new(error))
    }
}

impl Error {
    pub fn should_send_to_user(&self) -> bool {
        matches!(
            self,
            Error::GuildExpected
                | Error::AudioTimeout(_)
                | Error::LavaClientNotAvailable
                | Error::OpenAIUnavailable
                | Error::InvalidDuration
                | Error::EmptyResponse
                | Error::InvalidImage
                | Error::InvalidImageCount
                | Error::InvalidImageSize
        )
    }
}

pub async fn on_error(error: FrameworkError<'_, Data, Error>) {
    match error {
        FrameworkError::Command { error, ctx, .. } if error.should_send_to_user() => {
            crate::utils::send_err_msg(ctx, "Unable to run command", &error.to_string(), true)
                .await;
        }
        // Fallback to poise's default error handler
        _ => {
            if let Err(e) = poise_on_error(error).await {
                tracing::error!("Error while handling error: {}", e);
            }
        }
    }
}
