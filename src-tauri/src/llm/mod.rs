//! The local AI model that summaries and chat share: loading it through
//! llama.cpp, formatting prompts with its chat template, keeping it loaded,
//! and the worker thread that runs one summary or question at a time.

pub mod generator;
pub mod llama;
pub mod resident;
pub mod template;
pub mod worker;
