// SPDX-License-Identifier: AGPL-3.0-only
use serde::Serialize;

use crate::cli::{OutputFormat, EXIT_OUTPUT_ERROR};

#[derive(Debug, Clone)]
pub struct CommandFailure {
    pub exit_code: i32,
    pub message: String,
}

impl CommandFailure {
    pub fn new(exit_code: i32, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorPayload<'a> {
    command: &'a str,
    status: &'static str,
    exit_code: i32,
    error: &'a str,
}

pub fn emit_json<T: Serialize>(value: &T) -> Result<(), CommandFailure> {
    let rendered = serde_json::to_string_pretty(value).map_err(|error| {
        CommandFailure::new(
            EXIT_OUTPUT_ERROR,
            format!("failed to serialize JSON output: {error}"),
        )
    })?;
    println!("{rendered}");
    Ok(())
}

pub fn emit_error(command: &str, format: OutputFormat, exit_code: i32, message: &str) -> i32 {
    match format {
        OutputFormat::Text => {
            eprintln!("error: {message}");
            exit_code
        }
        OutputFormat::Json => {
            let payload = ErrorPayload {
                command,
                status: "error",
                exit_code,
                error: message,
            };

            match emit_json(&payload) {
                Ok(()) => exit_code,
                Err(error) => {
                    eprintln!("error: {}", error.message);
                    error.exit_code
                }
            }
        }
    }
}
