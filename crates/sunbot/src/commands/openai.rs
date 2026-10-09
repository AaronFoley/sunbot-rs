use std::vec;

use crate::{Context, Error};
use async_openai::types::chat::{
    ChatCompletionRequestMessageContentPartImageArgs,
    ChatCompletionRequestMessageContentPartTextArgs, ChatCompletionRequestUserMessageArgs,
    ChatCompletionRequestUserMessageContentPart, CreateChatCompletionRequestArgs, ReasoningEffort,
};
use async_openai::types::images::{
    CreateImageRequest, CreateImageRequestArgs, Image, ImageModel, ImageOutputFormat,
    ImageResponseFormat, ImageSize,
};
use base64::prelude::*;
use serenity::{
    all::{CreateAttachment, EditMessage},
    builder::CreateEmbed,
};

/// Ask a question to OpenAI
#[poise::command(slash_command)]
pub async fn askgpt(
    ctx: Context<'_>,
    #[description = "The prompt to send to OpenAI"] prompt: String,
    #[description = "Optional image to include with the question"] image: Option<
        serenity::all::Attachment,
    >,
) -> Result<(), Error> {
    let client = ctx
        .data()
        .openai_client
        .as_ref()
        .ok_or(Error::OpenAIUnavailable)?;

    let content = question_content(
        prompt,
        image.as_ref(),
        ctx.data().config.openai.askgpt.use_vision,
    )?;
    ctx.defer().await?;
    let mut request = CreateChatCompletionRequestArgs::default()
        .model(ctx.data().config.openai.askgpt.model.as_str())
        .messages([ChatCompletionRequestUserMessageArgs::default()
            .content(content)
            .build()?
            .into()])
        .max_completion_tokens(ctx.data().config.openai.askgpt.max_tokens)
        .build()?;

    configure_reasoning(&mut request);
    let resp = client.chat().create(request).await?;
    // The completion budget includes reasoning, so visible text may span messages.
    let text: Vec<char> = response_text(&resp)?.chars().collect();
    for chunk in text.chunks(1900) {
        ctx.reply(chunk.iter().collect::<String>()).await?;
    }
    Ok(())
}

#[derive(Debug, poise::ChoiceParameter)]
pub enum ImageSizeType {
    #[name = "1024x1024 (square)"]
    Square,
    #[name = "1536x1024 (landscape)"]
    Landscape,
    #[name = "1024x1536 (portrait)"]
    Portrait,
}

/// Generates an image using OpenAI
#[poise::command(slash_command)]
pub async fn genimage(
    ctx: Context<'_>,
    #[description = "The prompt to send to OpenAI"] prompt: String,
    #[description = "The size of the image to generate"] size: Option<ImageSizeType>,
    #[description = "The number of images to generate"] amount: Option<u8>,
) -> Result<(), Error> {
    let client = ctx
        .data()
        .openai_client
        .as_ref()
        .ok_or(Error::OpenAIUnavailable)?;

    if !(1..=10).contains(&amount.unwrap_or(1)) {
        return Err(Error::InvalidImageCount);
    }
    ctx.defer().await?;
    let model = match ctx.data().config.openai.genimage.model.as_str() {
        "dall-e-2" => ImageModel::DallE2,
        "dall-e-3" => ImageModel::DallE3,
        _ => ImageModel::Other(ctx.data().config.openai.genimage.model.clone()),
    };

    let mut embed = CreateEmbed::new()
        .title("Please wait generating images")
        .field("Prompt", prompt.as_str(), false)
        .field("Requester", format!("<@!{}>", ctx.author().id), false);

    let n = if model == ImageModel::DallE3 {
        if amount.unwrap_or(1) > 1 {
            embed = embed.description("NOTE: Only 1 image can be generated with DALL-E 3");
        }
        1
    } else {
        amount.unwrap_or(1)
    };

    let reply = ctx.send(poise::CreateReply::default().embed(embed)).await?;

    let request = image_request(
        model,
        prompt.as_str(),
        size.unwrap_or(ImageSizeType::Square),
        n,
    )?;

    let resp = client.images().generate(request).await?;

    if resp.data.is_empty() {
        return Err(Error::EmptyResponse);
    }
    let mut builder = EditMessage::new();

    let mut embeds = vec![CreateEmbed::new()
        // Setting this so that the embeds are joined together
        .url("https://openai.com")
        .field("Prompt", prompt.as_str(), false)
        .field("Requester", format!("<@!{}>", ctx.author().id), false)];

    for (pos, image) in resp.data.iter().enumerate() {
        match image.as_ref() {
            Image::Url {
                url,
                revised_prompt: _,
            } => {
                embeds.push(CreateEmbed::new().url("https://openai.com").image(url));
            }
            Image::B64Json {
                b64_json,
                revised_prompt: _,
            } => {
                let filename = format!("image-{}.png", pos);
                let data = BASE64_STANDARD.decode(b64_json.as_bytes())?;
                builder = builder.new_attachment(CreateAttachment::bytes(data, &filename));
                embeds.push(
                    CreateEmbed::new()
                        .url("https://openai.com")
                        .attachment(filename),
                );
            }
        }
    }

    builder = builder.embeds(embeds);
    // Calling this via the message object as the reply does not send new attachments
    reply.into_message().await?.edit(ctx, builder).await?;
    Ok(())
}

pub(crate) fn configure_reasoning(
    request: &mut async_openai::types::chat::CreateChatCompletionRequest,
) {
    // Keep conversational replies responsive; leave custom models' defaults alone.
    if request.model.starts_with("gpt-6.1-sol") {
        request.reasoning_effort = Some(ReasoningEffort::Low);
    } else if request.model.starts_with("gpt-6-luna") {
        request.reasoning_effort = Some(ReasoningEffort::None);
    }
}

fn image_request(
    model: ImageModel,
    prompt: &str,
    size: ImageSizeType,
    n: u8,
) -> Result<CreateImageRequest, Error> {
    let legacy = matches!(model, ImageModel::DallE2 | ImageModel::DallE3);
    let image_size = match (&model, size) {
        (_, ImageSizeType::Square) => ImageSize::S1024x1024,
        (ImageModel::DallE2, _) => return Err(Error::InvalidImageSize),
        (ImageModel::DallE3, ImageSizeType::Landscape) => ImageSize::S1792x1024,
        (ImageModel::DallE3, ImageSizeType::Portrait) => ImageSize::S1024x1792,
        (_, ImageSizeType::Landscape) => ImageSize::S1536x1024,
        (_, ImageSizeType::Portrait) => ImageSize::S1024x1536,
    };
    let mut request = CreateImageRequestArgs::default();
    request.model(model).n(n).prompt(prompt).size(image_size);
    if legacy {
        request.response_format(ImageResponseFormat::B64Json);
    } else {
        request.output_format(ImageOutputFormat::Png);
    }
    Ok(request.build()?)
}

pub(crate) fn response_text(
    response: &async_openai::types::chat::CreateChatCompletionResponse,
) -> Result<&str, Error> {
    response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_deref())
        .filter(|text| !text.trim().is_empty())
        .ok_or(Error::EmptyResponse)
}

fn question_content(
    prompt: String,
    image: Option<&serenity::all::Attachment>,
    vision: bool,
) -> Result<Vec<ChatCompletionRequestUserMessageContentPart>, Error> {
    let mut content = vec![ChatCompletionRequestMessageContentPartTextArgs::default()
        .text(prompt)
        .build()?
        .into()];
    if let Some(image) = image {
        if !vision
            || !image
                .content_type
                .as_deref()
                .is_some_and(|mime| mime.starts_with("image/"))
        {
            return Err(Error::InvalidImage);
        }
        content.push(
            ChatCompletionRequestMessageContentPartImageArgs::default()
                .image_url(image.url.as_str())
                .build()?
                .into(),
        );
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modern_requests_use_supported_parameters_and_legacy_images_still_work() {
        let request = image_request(
            ImageModel::Other("gpt-image-2.5-flare".into()),
            "test",
            ImageSizeType::Landscape,
            1,
        )
        .unwrap();
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["model"], "gpt-image-2.5-flare");
        assert_eq!(json["size"], "1536x1024");
        assert_eq!(json["output_format"], "png");
        assert!(json.get("response_format").is_none());
        let legacy = image_request(ImageModel::DallE3, "test", ImageSizeType::Portrait, 1).unwrap();
        let json = serde_json::to_value(legacy).unwrap();
        assert_eq!(json["size"], "1024x1792");
        assert_eq!(json["response_format"], "b64_json");
        assert!(json.get("output_format").is_none());
        for (model, effort) in [("gpt-6.1-sol", "low"), ("gpt-6-luna", "none")] {
            let mut request = CreateChatCompletionRequestArgs::default()
                .model(model)
                .messages([])
                .max_completion_tokens(100_u32)
                .build()
                .unwrap();
            configure_reasoning(&mut request);
            let json = serde_json::to_value(request).unwrap();
            assert_eq!(json["reasoning_effort"], effort);
            assert_eq!(json["max_completion_tokens"], 100);
            assert!(json.get("max_tokens").is_none());
        }
    }
    fn response(
        choices: serde_json::Value,
    ) -> async_openai::types::chat::CreateChatCompletionResponse {
        serde_json::from_value(serde_json::json!({
            "id": "test", "object": "chat.completion", "created": 0, "model": "test", "choices": choices
        })).unwrap()
    }
    #[test]
    fn empty_or_nontext_responses_return_errors() {
        assert!(response_text(&response(serde_json::json!([]))).is_err());
        for content in [
            serde_json::Value::Null,
            serde_json::json!(" "),
            serde_json::json!("hello"),
        ] {
            let response = response(
                serde_json::json!([{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}]),
            );
            assert_eq!(
                response_text(&response).is_ok(),
                content == serde_json::json!("hello")
            );
        }
    }
    #[test]
    fn vision_attachment_is_sent_only_when_enabled() {
        let image: serenity::all::Attachment = serde_json::from_value(serde_json::json!({
            "id": "1", "filename": "photo.png", "size": 1, "url": "https://example.com/photo.png",
            "proxy_url": "https://example.com/photo.png", "content_type": "image/png"
        }))
        .unwrap();
        let content = question_content("describe this".into(), Some(&image), true).unwrap();
        let content = serde_json::to_value(content).unwrap();
        assert_eq!(content[0]["text"], "describe this");
        assert_eq!(content[1]["image_url"]["url"], image.url);
        assert!(question_content("describe this".into(), Some(&image), false).is_err());
        let mut non_image = image;
        non_image.content_type = Some("text/plain".into());
        assert!(question_content("describe this".into(), Some(&non_image), true).is_err());
        assert_eq!(
            question_content("text".into(), None, false).unwrap().len(),
            1
        );
    }
}
