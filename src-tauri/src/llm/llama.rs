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

use super::generator::{GenerateError, Generator};
use super::template::{self, Message, SpecialTokens};

/// Cap on the context window: enough for a couple of hours of meeting in one
/// pass. Models with a large key-value cache set a lower limit in the registry.
pub const MAX_CONTEXT: usize = 32_768;
const BATCH: usize = 2048;

static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

fn backend() -> Result<&'static LlamaBackend, GenerateError> {
    BACKEND
        .get_or_init(|| {
            // Silence llama.cpp's and ggml's (Metal's) diagnostic output.
            // DIAGNOSTIC (#137): llama.cpp logs on.
            let _ = LogOptions::default();
            LlamaBackend::init().map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| GenerateError::Model(e.clone()))
}

/// Where a model runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Every layer, the key-value cache, and large operations on the GPU (Metal).
    Gpu,
    /// Everything on the CPU. For tests on machines whose GPU is slow, such as
    /// virtual CI runners; the app always uses the GPU.
    Cpu,
}

/// How each next token is chosen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sampling {
    pub temperature: f32,
    pub top_k: i32,
    pub top_p: f32,
    /// 0 turns min-p off.
    pub min_p: f32,
    /// 1 turns the repeat penalty off.
    pub repeat_penalty: f32,
}

impl Sampling {
    /// Earshot's settings: a low temperature keeps notes faithful to the
    /// transcript, and a mild repeat penalty stops small models from looping.
    pub const FAITHFUL: Sampling = Sampling {
        temperature: 0.3,
        top_k: 40,
        top_p: 0.9,
        min_p: 0.05,
        repeat_penalty: 1.1,
    };

    /// Google's published settings for Gemma 4, from its `generation_config.json`.
    pub const GEMMA_4: Sampling = Sampling {
        temperature: 1.0,
        top_k: 64,
        top_p: 0.95,
        min_p: 0.0,
        repeat_penalty: 1.0,
    };

    fn sampler(&self, n_vocab: i32) -> LlamaSampler {
        let mut chain = Vec::new();
        if self.repeat_penalty != 1.0 {
            chain.push(LlamaSampler::penalties(
                n_vocab,
                256,
                self.repeat_penalty,
                0.0,
                0.0,
            ));
        }
        chain.push(LlamaSampler::top_k(self.top_k));
        chain.push(LlamaSampler::top_p(self.top_p, 1));
        if self.min_p > 0.0 {
            chain.push(LlamaSampler::min_p(self.min_p, 1));
        }
        chain.push(LlamaSampler::temp(self.temperature));
        chain.push(LlamaSampler::dist(1234));
        LlamaSampler::chain_simple(chain)
    }
}

pub struct LlamaGenerator {
    model: LlamaModel,
    template: String,
    tokens: SpecialTokens,
    context_size: usize,
    placement: Placement,
    sampling: Sampling,
}

impl LlamaGenerator {
    /// Loads a GGUF model with every layer on the GPU.
    pub fn load(path: &Path) -> Result<Self, GenerateError> {
        Self::load_with_context_limit(path, MAX_CONTEXT)
    }

    /// Loads with the context capped below `MAX_CONTEXT`, for tests and for models
    /// whose key-value cache would otherwise outgrow the GPU's memory.
    pub fn load_with_context_limit(
        path: &Path,
        context_limit: usize,
    ) -> Result<Self, GenerateError> {
        Self::load_on(path, context_limit, Placement::Gpu)
    }

    /// Loads with the context capped at `context_limit`, running on `placement`.
    pub fn load_on(
        path: &Path,
        context_limit: usize,
        placement: Placement,
    ) -> Result<Self, GenerateError> {
        if !path.exists() {
            return Err(GenerateError::Model(format!(
                "the model file {} is missing",
                path.display()
            )));
        }
        let backend = backend()?;
        let gpu_layers = match placement {
            Placement::Gpu => u32::MAX,
            Placement::Cpu => 0,
        };
        let params = LlamaModelParams::default().with_n_gpu_layers(gpu_layers);
        let model = LlamaModel::load_from_file(backend, path, &params)
            .map_err(|e| GenerateError::Model(format!("couldn't load the model: {e}")))?;
        let template = model
            .chat_template(None)
            .map_err(|e| GenerateError::Model(format!("the model has no chat template: {e}")))?
            .to_string()
            .map_err(|e| {
                GenerateError::Model(format!("the chat template isn't valid text: {e}"))
            })?;
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
            placement,
            sampling: Sampling::FAITHFUL,
        })
    }

    /// Changes how tokens are chosen. The app uses `Sampling::FAITHFUL`.
    pub fn set_sampling(&mut self, sampling: Sampling) {
        self.sampling = sampling;
    }

    fn render(&self, messages: &[Message]) -> Result<String, GenerateError> {
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

    fn prompt_tokens(&mut self, messages: &[Message]) -> Result<usize, GenerateError> {
        Ok(self.tokenize_prompt(&self.render(messages)?).len())
    }

    fn count_tokens(&mut self, text: &str) -> Result<usize, GenerateError> {
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
    ) -> Result<String, GenerateError> {
        let prompt = self.tokenize_prompt(&self.render(messages)?);
        if prompt.len() + max_tokens > self.context_size {
            return Err(GenerateError::ContextTooSmall);
        }
        let backend = backend()?;
        let n_ctx = NonZeroU32::new((prompt.len() + max_tokens) as u32).expect("non-zero");
        let on_gpu = self.placement == Placement::Gpu;
        // DIAGNOSTIC (#137): thread override.
        let threads: Option<i32> = std::env::var("EARSHOT_TEST_THREADS")
            .ok()
            .and_then(|t| t.parse().ok());
        let params = LlamaContextParams::default()
            .with_n_ctx(Some(n_ctx))
            .with_n_batch(BATCH as u32)
            // Without these, llama.cpp still moves the cache and large operations to the GPU.
            .with_offload_kqv(on_gpu)
            .with_op_offload(on_gpu);
        let params = match threads {
            Some(n) => params.with_n_threads(n).with_n_threads_batch(n),
            None => params,
        };
        eprintln!("DIAGNOSTIC threads={:?}", threads);
        let mut ctx = self
            .model
            .new_context(backend, params)
            .map_err(|e| GenerateError::Model(format!("couldn't create a context: {e}")))?;

        let model_error = |e: &dyn std::fmt::Display| GenerateError::Model(e.to_string());
        let mut batch = LlamaBatch::new(BATCH, 1);
        let last = prompt.len() - 1;
        for (chunk_index, chunk) in prompt.chunks(BATCH).enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(GenerateError::Cancelled);
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

        let vocab = self.model.vocab();
        let mut sampler = self.sampling.sampler(self.model.n_vocab());
        let mut stream = Utf8Stream::default();
        let mut output = String::new();
        for pos in (prompt.len()..).take(max_tokens) {
            if cancel.load(Ordering::Relaxed) {
                return Err(GenerateError::Cancelled);
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
