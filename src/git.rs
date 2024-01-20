use crate::config_loader::Config;
use anyhow::{ensure, Context, Result};
use async_openai::types::{
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    ChatCompletionResponseMessage, CreateChatCompletionRequestArgs,
};
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Deserialize)]
struct CommitMessageCandidate {
    message: ChatCompletionResponseMessage, // This stores a single commit message candidate generated by the model
}

#[derive(Deserialize)]
struct CommitMessageCandidates {
    choices: Vec<CommitMessageCandidate>, // This stores all the commit message candidates generated by the model
}

async fn run_git_command(args: &[&str]) -> Result<String> {
    let response = Command::new("git")
        .args(args)
        .output()
        .await
        .context("Failed to execute Git command.")?;

    ensure!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );

    String::from_utf8(response.stdout).context("Failed to decode output of the Git command.")
}

pub async fn git_diff() -> Result<String> {
    let git_diffs = run_git_command(&[
        "--no-pager",
        "diff",
        "--staged",
        "--minimal",
        "--no-color",
        "--no-ext-diff",
        "--",
        ":!*.lock",
    ])
    .await?
    .trim()
    .to_string();

    ensure!(
        !git_diffs.is_empty(),
        "There are no staged changes to commit."
    );

    Ok(git_diffs)
}

pub async fn generate_commit_message(
    http_client: &Client,
    config: &Config,
    diff: &str,
) -> Result<String> {
    let payload = CreateChatCompletionRequestArgs::default()
        .max_tokens(config.max_chars)
        .model(&config.model_name)
        .messages([
            ChatCompletionRequestSystemMessageArgs::default()
                .content(&config.system_prompt)
                .build()?
                .into(),
            ChatCompletionRequestUserMessageArgs::default()
                .content(config.user_prompt.replace("{}", diff))
                .build()?
                .into(),
        ])
        .build()
        .context("Failed to construct the request payload")?;

    let response = http_client
        .post(format!("{}/chat/completions", &config.api_base_url))
        .bearer_auth(&config.api_key)
        .json(&payload)
        .send()
        .await
        .context("Failed to send the request to the Inference API provider")?
        .error_for_status()?
        .json::<CommitMessageCandidates>()
        .await
        .context("Failed to parse the response from the Inference API provider")?;

    let commit_message = response
        .choices
        .first() // Only the first generated commit message is used
        .context("No commit messages generated")?
        .message
        .content
        .as_ref()
        .context("No commit messages generated")?;

    // Post-process the generated commit message to keep only the first line and remove leading and trailing backticks
    let regex_matches = Regex::new(r"(?m)^\s*(?:`\s*(.+?)\s*`|(.+?))\s*$")?
        .captures(commit_message)
        .context("Failed to post-process the generated commit message")?;

    let commit_message = regex_matches
        .get(1)
        .or(regex_matches.get(2))
        .context("Failed to post-process the generated commit message")?
        .as_str()
        .to_string();

    Ok(commit_message)
}
