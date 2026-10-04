//! The `Generator` backed by llama.cpp, with Metal acceleration on macOS.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use llama_cpp_2::LogOptions;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;

use super::run::{Generator, SummaryError};
use super::template::{self, Message, SpecialTokens};

/// Cap on the context window: enough for a couple of hours of meeting in one
/// pass while keeping the key-value cache to a few hundred megabytes.
pub const MAX_CONTEXT: usize = 32_768;
const BATCH: usize = 2048;

static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

fn backend() -> Result<&'static LlamaBackend, SummaryError> {
    BACKEND
        .get_or_init(|| {
            // Silence llama.cpp's and ggml's (Metal's) diagnostic output.
            llama_cpp_2::send_logs_to_tracing(LogOptions::default().with_logs_enabled(false));
            LlamaBackend::init().map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| SummaryError::Model(e.clone()))
}

pub struct LlamaGenerator {
    model: LlamaModel,
    template: String,
    tokens: SpecialTokens,
    context_size: usize,
}

impl LlamaGenerator {
    /// Loads a GGUF model with every layer on the GPU.
    pub fn load(path: &Path) -> Result<Self, SummaryError> {
        Self::load_with_context_limit(path, MAX_CONTEXT)
    }

    /// Loads with the context capped below `MAX_CONTEXT`, for tests and small machines.
    pub fn load_with_context_limit(
        path: &Path,
        context_limit: usize,
    ) -> Result<Self, SummaryError> {
        if !path.exists() {
            return Err(SummaryError::Model(format!(
                "the model file {} is missing",
                path.display()
            )));
        }
        let backend = backend()?;
        let params = LlamaModelParams::default().with_n_gpu_layers(u32::MAX);
        let model = LlamaModel::load_from_file(backend, path, &params)
            .map_err(|e| SummaryError::Model(format!("couldn't load the model: {e}")))?;
        let template = model
            .chat_template(None)
            .map_err(|e| SummaryError::Model(format!("the model has no chat template: {e}")))?
            .to_string()
            .map_err(|e| SummaryError::Model(format!("the chat template isn't valid text: {e}")))?;
        let vocab = model.vocab();
        let piece =
            |token| String::from_utf8_lossy(&vocab.token_to_piece(token, true, None)).into_owned();
        let tokens = SpecialTokens {
            bos: piece(vocab.bos()),
            eos: piece(vocab.eos()),
        };
        let context_size = (model.n_ctx_train() as usize)
            .min(MAX_CONTEXT)
            .min(context_limit);
        Ok(Self {
            model,
            template,
            tokens,
            context_size,
        })
    }

    fn render(&self, messages: &[Message]) -> Result<String, SummaryError> {
        Ok(template::render(&self.template, messages, &self.tokens)?)
    }

    fn tokenize_prompt(&self, prompt: &str) -> Vec<LlamaToken> {
        let vocab = self.model.vocab();
        // Templates usually write the BOS token themselves; add it only if missing.
        let add_bos = vocab.should_add_bos() && !prompt.starts_with(&self.tokens.bos);
        vocab.tokenize(prompt.as_bytes(), add_bos, true)
    }
}

impl Generator for LlamaGenerator {
    fn context_size(&self) -> usize {
        self.context_size
    }

    fn prompt_tokens(&mut self, messages: &[Message]) -> Result<usize, SummaryError> {
        Ok(self.tokenize_prompt(&self.render(messages)?).len())
    }

    fn count_tokens(&mut self, text: &str) -> Result<usize, SummaryError> {
        Ok(self
            .model
            .vocab()
            .tokenize(text.as_bytes(), false, false)
            .len())
    }

    fn generate(
        &mut self,
        messages: &[Message],
        max_tokens: usize,
        cancel: &AtomicBool,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<String, SummaryError> {
        let prompt = self.tokenize_prompt(&self.render(messages)?);
        if prompt.len() + max_tokens > self.context_size {
            return Err(SummaryError::ContextTooSmall);
        }
        let backend = backend()?;
        let n_ctx = NonZeroU32::new((prompt.len() + max_tokens) as u32).expect("non-zero");
        let params = LlamaContextParams::default()
            .with_n_ctx(Some(n_ctx))
            .with_n_batch(BATCH as u32);
        let mut ctx = self
            .model
            .new_context(backend, params)
            .map_err(|e| SummaryError::Model(format!("couldn't create a context: {e}")))?;

        let model_error = |e: &dyn std::fmt::Display| SummaryError::Model(e.to_string());
        let mut batch = LlamaBatch::new(BATCH, 1);
        let last = prompt.len() - 1;
        for (chunk_index, chunk) in prompt.chunks(BATCH).enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(SummaryError::Cancelled);
            }
            batch.clear();
            for (offset, &token) in chunk.iter().enumerate() {
                let pos = chunk_index * BATCH + offset;
                batch
                    .add(token, pos as i32, &[0], pos == last)
                    .map_err(|e| model_error(&e))?;
            }
            ctx.decode(&mut batch).map_err(|e| model_error(&e))?;
        }

        // Low temperature keeps notes faithful to the transcript; a mild repeat
        // penalty stops small models from looping.
        let vocab = self.model.vocab();
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::penalties(self.model.n_vocab(), 256, 1.1, 0.0, 0.0),
            LlamaSampler::top_k(40),
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::min_p(0.05, 1),
            LlamaSampler::temp(0.3),
            LlamaSampler::dist(1234),
        ]);
        let mut stream = Utf8Stream::default();
        let mut output = String::new();
        for pos in (prompt.len()..).take(max_tokens) {
            if cancel.load(Ordering::Relaxed) {
                return Err(SummaryError::Cancelled);
            }
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            if vocab.is_eog(token) {
                break;
            }
            let text = stream.push(&vocab.token_to_piece(token, false, None));
            if !text.is_empty() {
                on_text(&text);
                output.push_str(&text);
            }
            batch.clear();
            batch
                .add(token, pos as i32, &[0], true)
                .map_err(|e| model_error(&e))?;
            ctx.decode(&mut batch).map_err(|e| model_error(&e))?;
        }
        let rest = stream.finish();
        if !rest.is_empty() {
            on_text(&rest);
            output.push_str(&rest);
        }
        Ok(output.trim().to_owned())
    }
}

/// Reassembles UTF-8 text from token pieces, which can split a character.
#[derive(Default)]
struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.pending) {
            Ok(_) => self.pending.len(),
            // An incomplete character at the end waits for the next piece.
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            // Invalid bytes can't become valid later; replace them.
            Err(_) => {
                let text = String::from_utf8_lossy(&self.pending).into_owned();
                self.pending.clear();
                return text;
            }
        };
        String::from_utf8(self.pending.drain(..valid).collect()).expect("validated")
    }

    fn finish(self) -> String {
        String::from_utf8_lossy(&self.pending).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_back_a_split_character_until_it_completes() {
        let mut stream = Utf8Stream::default();
        let euro = "€".as_bytes(); // three bytes

        assert_eq!(stream.push(b"a "), "a ");
        assert_eq!(stream.push(&euro[..2]), "");
        assert_eq!(stream.push(&euro[2..]), "€");
        assert_eq!(stream.finish(), "");
    }

    #[test]
    fn replaces_invalid_bytes_instead_of_stalling() {
        let mut stream = Utf8Stream::default();

        assert_eq!(stream.push(&[b'a', 0xFF, b'b']), "a\u{FFFD}b");
    }

    #[test]
    fn flushes_a_dangling_partial_character_at_the_end() {
        let mut stream = Utf8Stream::default();

        assert_eq!(stream.push(&"é".as_bytes()[..1]), "");
        assert_eq!(stream.finish(), "\u{FFFD}");
    }
}
