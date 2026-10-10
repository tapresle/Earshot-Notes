//! What summaries and chat need from a loaded model.

use std::sync::atomic::AtomicBool;

use super::template::{Message, TemplateError};

#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    #[error("the summary was cancelled")]
    Cancelled,
    #[error("the model's context is too small to summarize this meeting")]
    ContextTooSmall,
    #[error(transparent)]
    Template(#[from] TemplateError),
    #[error("the model failed: {0}")]
    Model(String),
}

/// A loaded model that can format prompts and generate text.
pub trait Generator {
    /// Total tokens the model can attend to, prompt plus output.
    fn context_size(&self) -> usize;
    /// Tokens the rendered prompt for these messages takes.
    fn prompt_tokens(&mut self, messages: &[Message]) -> Result<usize, GenerateError>;
    fn count_tokens(&mut self, text: &str) -> Result<usize, GenerateError>;
    /// Generates a reply, passing each piece of text to `on_text` as it arrives.
    /// Returns `GenerateError::Cancelled` soon after `cancel` is set.
    fn generate(
        &mut self,
        messages: &[Message],
        max_tokens: usize,
        cancel: &AtomicBool,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<String, GenerateError>;
}
