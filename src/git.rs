use crate::config::Config;
use anyhow::{ensure, Context, Result};
use async_openai::types::{
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    ChatCompletionResponseMessage, CreateChatCompletionRequestArgs,
};
use inquire::{required, Text};
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use tokio::process::Command;
use which::which;

/// Stores a single commit message candidate generated by the model
#[derive(Deserialize)]
struct CommitMessageCandidate {
    message: ChatCompletionResponseMessage,
}

/// Stores all the commit message candidates generated by the model
#[derive(Deserialize)]
struct CommitMessageCandidates {
    choices: Vec<CommitMessageCandidate>,
}

/// Asynchronously executes a Git command with the specified arguments.
///
/// Invokes a Git command with the given arguments, waits for the command to complete,
/// and returns the command's standard output as a string if successful.
///
/// # Arguments
///
/// * `args` - A slice of string references representing the arguments for the Git command.
///
/// # Returns
///
/// Returns a `Result` containing the standard output of the Git command as a string on success,
/// or an error if there were issues executing the command or decoding the output.
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

/// Asynchronously performs Git-related checks to ensure Git is installed and the current directory is a Git repository.
///
/// Checks whether Git is installed and if the current directory is a Git repository by invoking
/// relevant Git commands. Returns a `Result` indicating success or an error if Git is not installed
/// or the current directory is not a Git repository.
///
/// # Returns
///
/// Returns a `Result` indicating success on passing Git checks or an error with a relevant message if checks fail.
pub async fn git_checks() -> Result<()> {
    which("git").context("Git may not be installed.")?;

    run_git_command(&["rev-parse", "--is-inside-work-tree"])
        .await
        .context("The current directory is not a Git respository.")?;

    Ok(())
}

pub async fn git_commit(commit_message: &str) -> Result<String> {
    let result = run_git_command(&["commit", "-m", commit_message])
        .await?
        .trim()
        .to_string();

    Ok(result)
}

/// Asynchronously retrieves the staged Git differences.
///
/// Executes the Git command to retrieve the staged differences, ensuring that there are staged changes
/// to commit. Returns the staged differences as a string if successful.
///
/// # Returns
///
/// Returns a `Result` containing the staged Git differences as a string on success,
/// or an error if there were issues executing the Git command or if there are no staged changes.
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

/// Asynchronously generates a commit message using the provided HTTP client, configuration, and Git differences.
///
/// Constructs a request payload for the API based on the provided configuration and staged Git differences.
/// Sends the request to the API provider, retrieves and parses the response, and extracts the generated
/// commit message. Post-processes the commit message to keep only the first line and remove leading and trailing backticks.
///
/// # Arguments
///
/// * `http_client` - A reference to the Reqwest HTTP client used to send requests to the API.
/// * `config` - A reference to the configuration containing API details, model information, and prompt contents.
/// * `diff` - A reference to the staged Git differences to be included in the user prompt.
///
/// # Returns
///
/// Returns a `Result` containing the generated commit message as a string on success,
/// or an error if there were issues constructing the request, sending it, or processing the response.
pub async fn generate_commit_message(
    http_client: &Client,
    config: &Config,
    diff: &str,
) -> Result<String> {
    let payload = CreateChatCompletionRequestArgs::default()
        .max_tokens(config.max_chars)
        .model(&config.git_model_name)
        .messages([
            ChatCompletionRequestSystemMessageArgs::default()
                .content(&config.commit_prompt)
                .build()?
                .into(),
            ChatCompletionRequestUserMessageArgs::default()
                .content(config.diff_prompt.replace("{}", diff))
                .build()?
                .into(),
        ])
        .build()
        .context("Failed to construct the request payload")?;

    let response = http_client
        .post(format!("{}/chat/completions", &config.git_api_base_url))
        .bearer_auth(&config.api_key)
        .json(&payload)
        .send()
        .await
        .context("Failed to send the request to the API provider")?
        .error_for_status()?
        .json::<CommitMessageCandidates>()
        .await
        .context("Failed to parse the response from the API provider")?;

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

pub fn edit_commit_message(generated_commit_message: &str) -> Result<String> {
    // Ask user to edit the generated commit message if needed
    let edited_commit_message = Text::new("Your generated commit message:")
        .with_initial_value(&generated_commit_message)
        .with_validator(required!(
            "Please provide a commit message to create a commit"
        ))
        .with_help_message(
            "Press Enter to create a new commit with the current message or ESC to cancel",
        )
        .prompt()?;

    Ok(edited_commit_message)
}
