use std::slice::SliceIndex;

use anyhow::{Context, Result};
use async_openai::types::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs, ChatCompletionResponseMessage,
    CreateChatCompletionRequestArgs,
};

use regex::Regex;
use reqwest::Client;
use serde::Deserialize;

use crate::app_config::config::Config;

/// Stores a single summary message candidate generated by the model
#[derive(Deserialize)]
struct SlideResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: MessageContent,
}

#[derive(Deserialize)]
struct MessageContent {
    content: String,
}

pub async fn generate_slide_summary(
    http_client: &Client,
    config: &Config,
    file_text_list: Vec<String>,
) -> Result<String> {
    
    let mut chat_completion_request_system_message_args_list: Vec<ChatCompletionRequestMessage> = Vec::new();

    chat_completion_request_system_message_args_list.push(
        ChatCompletionRequestSystemMessageArgs::default()
            .content(&config.slides_prompt)
            .build()?
            .into(),
    );

    for file_text in file_text_list {

        chat_completion_request_system_message_args_list.push(
            ChatCompletionRequestUserMessageArgs::default()
                .content(file_text)
                .build()?
                .into(),
        )
    }

    let payload = CreateChatCompletionRequestArgs::default()
        .max_tokens(config.max_chars)
        .model(&config.git_model_name)
        .messages(chat_completion_request_system_message_args_list)
        .build()
        .context("Failed to construct the request payload")?;

    let response = http_client
        .post(format!("{}", &config.git_api_base_url))
        .bearer_auth(&config.api_key)
        .json(&payload)
        .send()
        .await
        .context("Failed to send the request to the API provider")?;
        
    let summary_message = response
        .json::<SlideResponse>()
        .await?
        .choices
        .first() // Only the first generated summary message is used
        .unwrap() // Unwrap the Option<&code_summarizer::Choice> to access the Choice struct
        .message
        .content
        .clone();

    println!("{}", summary_message);

    

    Ok(summary_message)
}
