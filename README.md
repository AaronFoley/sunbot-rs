# Sunbot-rs

A simple discord bot using [serenity-rs]
This bot was written to add some fun to my personal discord server, and it not intended to be run in production.

OpenAI defaults are `gpt-6.1-sol` for `/askgpt`, `gpt-6-luna` for automatic replies, and `gpt-image-2.5-flare` for images. Explicit model settings in your configuration override these defaults. The chat `max_tokens` setting now controls the completion budget, including reasoning; `/askgpt` defaults to 4096 and splits long replies into multiple Discord messages. Image sizes are square, landscape, and portrait; re-register slash commands to update these choices.
