//! Local LLM files: the recommended models, downloading them, and importing a GGUF from disk.

pub mod download;
pub mod manager;
pub mod transcription;

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::llm::llama::MAX_CONTEXT;
use crate::store::{ModelRecord, ModelSource, Store, StoreError};

/// The llama-cpp-2 version this build runs, recorded as the minimum runtime of
/// models known to work with it. Must match the exact version Cargo.toml pins.
pub const RUNTIME_VERSION: &str = "0.1.158";

/// A model the app knows how to fetch and verify.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownModel {
    pub id: &'static str,
    pub display_name: &'static str,
    pub file_name: &'static str,
    /// The quantization of the weights, such as `Q4_K_M`.
    pub quantization: &'static str,
    pub repo: &'static str,
    pub revision: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
    pub version: &'static str,
    pub min_runtime: &'static str,
    /// One line that helps someone choose between models.
    pub description: &'static str,
    /// Largest context, in tokens, to run the model with. Keeps the key-value
    /// cache of larger models inside the memory Metal can use on a 16 GB Mac;
    /// longer meetings are condensed in parts instead.
    pub context_limit: usize,
}

impl KnownModel {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.revision, self.file_name
        )
    }

    /// The repository's page on Hugging Face, with the model card.
    pub fn page_url(&self) -> String {
        format!("https://huggingface.co/{}", self.repo)
    }
}

/// The recommended local AI model (#96).
pub const QWEN_3_5_4B: KnownModel = KnownModel {
    id: "qwen3.5-4b-q4_k_m",
    display_name: "Qwen3.5 4B",
    file_name: "Qwen3.5-4B-Q4_K_M.gguf",
    quantization: "Q4_K_M",
    repo: "unsloth/Qwen3.5-4B-GGUF",
    // The commit that last changed this file, which its Hugging Face page shows.
    revision: "720bb031aae5488eae5d6a78768e6d826662b2ae",
    sha256: "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
    size_bytes: 2_740_937_888,
    version: "Qwen3.5-4B Q4_K_M (unsloth 720bb03)",
    min_runtime: "0.1.158",
    description: "Fast and light, with detailed summaries. A good fit for most meetings.",
    context_limit: MAX_CONTEXT,
};

pub const GEMMA_4_E4B_QAT: KnownModel = KnownModel {
    id: "gemma-4-e4b-it-qat-ud-q4_k_xl",
    display_name: "Gemma 4 E4B",
    file_name: "gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf",
    quantization: "UD-Q4_K_XL",
    repo: "unsloth/gemma-4-E4B-it-qat-GGUF",
    revision: "8c5a9e4fd5482e2be20fe0bf013b4c262a8f4265",
    sha256: "df0fd4ee07072c607c29a0a1cb4f98918426cca12f45a2776bdd6ee6d09a4de3",
    size_bytes: 4_215_695_776,
    version: "gemma-4-E4B-it-qat UD-Q4_K_XL (unsloth 8c5a9e4)",
    min_runtime: "0.1.158",
    description: "Google's model, with well-organized summaries. A little slower.",
    context_limit: MAX_CONTEXT,
};

pub const MINISTRAL_3_8B: KnownModel = KnownModel {
    id: "ministral-3-8b-instruct-2512-q4_k_m",
    display_name: "Ministral 3 8B",
    file_name: "Ministral-3-8B-Instruct-2512-Q4_K_M.gguf",
    quantization: "Q4_K_M",
    repo: "unsloth/Ministral-3-8B-Instruct-2512-GGUF",
    revision: "1b83c277dcaa1f736c25cf1e3cc4b48110697df2",
    sha256: "5dbc3647eb563b9f8d3c70ec3d906cce84b86bb35c5e0b8a36e7df3937ab7174",
    size_bytes: 5_198_386_720,
    version: "Ministral-3-8B-Instruct-2512 Q4_K_M (unsloth 1b83c27)",
    min_runtime: "0.1.158",
    description: "Very complete summaries. The slowest option, and uses more memory.",
    context_limit: 16_384,
};

pub const QWEN_3_5_9B: KnownModel = KnownModel {
    id: "qwen3.5-9b-q4_k_m",
    display_name: "Qwen3.5 9B",
    file_name: "Qwen3.5-9B-Q4_K_M.gguf",
    quantization: "Q4_K_M",
    repo: "unsloth/Qwen3.5-9B-GGUF",
    revision: "99a1b2185534379e6e8b5ec869da25d3e7b3f73c",
    sha256: "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8",
    size_bytes: 5_680_522_464,
    version: "Qwen3.5-9B Q4_K_M (unsloth 99a1b21)",
    min_runtime: "0.1.158",
    description: "The most detailed summaries. Slower, and uses more memory.",
    context_limit: MAX_CONTEXT,
};

/// Whisper large-v3-turbo, the optional transcription model (#25). It isn't a
/// summary model, so it's not in `KNOWN_MODELS`; `models::transcription`
/// manages it. `min_runtime` is the whisper.cpp release it was measured with,
/// and `context_limit` doesn't apply.
pub const WHISPER_TURBO: KnownModel = KnownModel {
    id: "whisper-large-v3-turbo-q5_0",
    display_name: "Whisper",
    file_name: "ggml-large-v3-turbo-q5_0.bin",
    quantization: "Q5_0",
    repo: "ggerganov/whisper.cpp",
    // The commit that added this file, which its Hugging Face page shows.
    revision: "98aa99a0a9db05ae2342309f5096248665f7cba3",
    sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    size_bytes: 574_041_195,
    version: "ggml-large-v3-turbo-q5_0 (ggerganov/whisper.cpp 98aa99a)",
    min_runtime: "whisper.cpp 1.9.5",
    description: "More accurate, especially for the other side of calls and technical terms. Heavier: it stays loaded while the app is open, using about 1 GB of memory.",
    context_limit: 0,
};

/// Every downloadable model, in the order the app lists them (see `display_order`).
pub const KNOWN_MODELS: &[KnownModel] =
    &[QWEN_3_5_4B, GEMMA_4_E4B_QAT, MINISTRAL_3_8B, QWEN_3_5_9B];

/// The order the app lists models in: the recommended model first, then
/// smallest first.
pub fn display_order(id: &str, size_bytes: u64) -> (bool, u64) {
    (id != QWEN_3_5_4B.id, size_bytes)
}

/// Sorts installed models into `display_order`.
pub fn sort_for_display(models: &mut [ModelRecord]) {
    models.sort_by_key(|m| display_order(&m.id, m.size_bytes.max(0) as u64));
}

/// A known summary or transcription model.
pub fn known_by_id(id: &str) -> Option<&'static KnownModel> {
    KNOWN_MODELS
        .iter()
        .chain(std::iter::once(&WHISPER_TURBO))
        .find(|k| k.id == id)
}

/// The context limit for an installed model: its registry limit when it's a
/// known model, or the app-wide cap for anything imported.
pub fn context_limit(sha256: &str) -> usize {
    KNOWN_MODELS
        .iter()
        .find(|k| k.sha256 == sha256)
        .map_or(MAX_CONTEXT, |k| k.context_limit)
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("that file isn't a GGUF model")]
    NotAModel,
    #[error("couldn't read or copy the model: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Copies a GGUF file into `models_dir` and records it. It becomes the summary
/// model only when no other model is installed. Importing a file that's
/// already installed returns the existing record.
pub fn import(
    source: &Path,
    models_dir: &Path,
    store: &Store,
    known: &[KnownModel],
) -> Result<ModelRecord, ImportError> {
    if !has_gguf_header(source)? {
        return Err(ImportError::NotAModel);
    }
    let sha256 = sha256_file(source)?;
    if let Some(existing) = store.model_by_sha256(&sha256)? {
        return Ok(existing);
    }

    let known = known.iter().find(|k| k.sha256 == sha256);
    let file_name = match known {
        Some(k) => k.file_name.to_owned(),
        None => source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "model.gguf".into()),
    };
    std::fs::create_dir_all(models_dir)?;
    let dest = unique_path(&models_dir.join(file_name));
    std::fs::copy(source, &dest)?;

    let record = ModelRecord {
        id: known.map_or_else(|| format!("local-{}", &sha256[..12]), |k| k.id.to_owned()),
        display_name: known.map_or_else(
            || {
                source
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Imported model".into())
            },
            |k| k.display_name.to_owned(),
        ),
        file_path: dest.to_string_lossy().into_owned(),
        size_bytes: std::fs::metadata(&dest)?.len() as i64,
        sha256,
        source: ModelSource::Import,
        source_url: known.map(KnownModel::url),
        version: known.map(|k| k.version.to_owned()),
        min_runtime: known.map(|k| k.min_runtime.to_owned()),
        added_at: 0,
    };
    let saved = store.add_model_keeping_selection(&record);
    if saved.is_err() {
        let _ = std::fs::remove_file(&dest);
    }
    Ok(saved?)
}

#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    #[error("couldn't delete the model file: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Deletes an installed model's file and forgets the model. Only a file inside
/// `models_dir` is deleted; a record that points anywhere else is just forgotten.
pub fn remove(id: &str, models_dir: &Path, store: &Store) -> Result<ModelRecord, RemoveError> {
    let model = store
        .model_by_id(id)?
        .ok_or_else(|| StoreError::ModelNotFound(id.to_owned()))?;
    let path = Path::new(&model.file_path);
    if is_inside(path, models_dir) {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    store.delete_model(id)?;
    Ok(model)
}

/// Whether `path` resolves to a file under `dir`, following symlinks and `..`.
fn is_inside(path: &Path, dir: &Path) -> bool {
    match (path.canonicalize(), dir.canonicalize()) {
        (Ok(path), Ok(dir)) => path.starts_with(dir),
        _ => false,
    }
}

fn has_gguf_header(path: &Path) -> io::Result<bool> {
    let mut magic = [0u8; 4];
    match File::open(path)?.read_exact(&mut magic) {
        Ok(()) => Ok(&magic == b"GGUF"),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// `name.gguf`, or `name-2.gguf` and so on if that's taken.
fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| path.with_file_name(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_gguf(dir: &Path, name: &str, body: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, [b"GGUF".as_slice(), body].concat()).unwrap();
        path
    }

    #[test]
    fn imports_copies_and_selects_a_first_model() {
        let source_dir = tempfile::tempdir().unwrap();
        let models_dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let source = fake_gguf(source_dir.path(), "my-model.gguf", b"weights");

        let record = import(&source, models_dir.path(), &store, &[]).unwrap();

        assert_eq!(record.display_name, "my-model");
        assert_eq!(record.source, ModelSource::Import);
        assert_eq!(record.version, None);
        assert!(record.id.starts_with("local-"));
        assert_eq!(record.sha256, sha256_file(&source).unwrap());
        assert_eq!(
            std::fs::read(&record.file_path).unwrap(),
            std::fs::read(&source).unwrap()
        );
        assert_eq!(store.selected_model().unwrap(), Some(record));
    }

    #[test]
    fn recognizes_a_known_model_by_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let source = fake_gguf(dir.path(), "downloaded.gguf", b"known");
        let sha256: &'static str = Box::leak(sha256_file(&source).unwrap().into_boxed_str());
        let known = KnownModel {
            sha256,
            ..QWEN_3_5_4B
        };

        let record = import(&source, &dir.path().join("models"), &store, &[known]).unwrap();

        assert_eq!(record.id, QWEN_3_5_4B.id);
        assert_eq!(record.display_name, "Qwen3.5 4B");
        assert!(record.file_path.ends_with(QWEN_3_5_4B.file_name));
        assert_eq!(record.version.as_deref(), Some(QWEN_3_5_4B.version));
        assert_eq!(record.min_runtime.as_deref(), Some("0.1.158"));
        assert_eq!(record.source_url, Some(QWEN_3_5_4B.url()));
    }

    #[test]
    fn keeps_the_selected_model_when_importing_another() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let models = dir.path().join("models");
        let first = import(&fake_gguf(dir.path(), "a.gguf", b"a"), &models, &store, &[]).unwrap();

        import(&fake_gguf(dir.path(), "b.gguf", b"b"), &models, &store, &[]).unwrap();

        assert_eq!(store.selected_model().unwrap(), Some(first));
    }

    #[test]
    fn reimporting_an_installed_model_does_not_select_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let models = dir.path().join("models");
        let a = fake_gguf(dir.path(), "a.gguf", b"a");
        import(&a, &models, &store, &[]).unwrap();
        let b = import(&fake_gguf(dir.path(), "b.gguf", b"b"), &models, &store, &[]).unwrap();
        store.select_model(&b.id).unwrap();

        import(&a, &models, &store, &[]).unwrap();

        assert_eq!(store.selected_model().unwrap(), Some(b));
    }

    #[test]
    fn importing_the_same_file_twice_reuses_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let source = fake_gguf(dir.path(), "m.gguf", b"same");

        let first = import(&source, &dir.path().join("models"), &store, &[]).unwrap();
        let second = import(&source, &dir.path().join("models"), &store, &[]).unwrap();

        assert_eq!(first, second);
        assert_eq!(store.list_models().unwrap().len(), 1);
    }

    #[test]
    fn keeps_both_files_when_names_collide() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let models = dir.path().join("models");
        let a = fake_gguf(dir.path(), "m.gguf", b"a");
        import(&a, &models, &store, &[]).unwrap();
        let other = tempfile::tempdir().unwrap();
        let b = fake_gguf(other.path(), "m.gguf", b"b");

        let second = import(&b, &models, &store, &[]).unwrap();

        assert!(
            second.file_path.ends_with("m-2.gguf"),
            "{}",
            second.file_path
        );
    }

    #[test]
    fn removes_the_file_and_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let models = dir.path().join("models");
        let source = fake_gguf(dir.path(), "m.gguf", b"weights");
        let record = import(&source, &models, &store, &[]).unwrap();

        let removed = remove(&record.id, &models, &store).unwrap();

        assert_eq!(removed, record);
        assert!(!Path::new(&record.file_path).exists());
        assert!(source.exists(), "the original file is left alone");
        assert!(store.list_models().unwrap().is_empty());
        assert_eq!(store.selected_model().unwrap(), None);
    }

    #[test]
    fn forgets_a_model_whose_file_is_already_gone() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let models = dir.path().join("models");
        let record = import(&fake_gguf(dir.path(), "m.gguf", b"w"), &models, &store, &[]).unwrap();
        std::fs::remove_file(&record.file_path).unwrap();

        remove(&record.id, &models, &store).unwrap();

        assert!(store.list_models().unwrap().is_empty());
    }

    #[test]
    fn never_deletes_a_file_outside_the_models_folder() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let models = dir.path().join("models");
        std::fs::create_dir_all(&models).unwrap();
        let outside = fake_gguf(dir.path(), "outside.gguf", b"keep");
        let sneaky = models.join("..").join("outside.gguf");
        let record = store
            .add_model(&ModelRecord {
                id: "elsewhere".into(),
                display_name: "Elsewhere".into(),
                file_path: sneaky.to_string_lossy().into_owned(),
                size_bytes: 8,
                sha256: "e".repeat(64),
                source: ModelSource::Import,
                source_url: None,
                version: None,
                min_runtime: None,
                added_at: 0,
            })
            .unwrap();

        remove(&record.id, &models, &store).unwrap();

        assert!(outside.exists());
        assert!(store.list_models().unwrap().is_empty());
    }

    #[test]
    fn refuses_to_remove_an_unknown_model() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();

        assert!(matches!(
            remove("ghost", dir.path(), &store),
            Err(RemoveError::Store(StoreError::ModelNotFound(_)))
        ));
    }

    #[test]
    fn rejects_a_file_that_is_not_gguf() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let source = dir.path().join("notes.txt");
        std::fs::write(&source, b"hello").unwrap();

        assert!(matches!(
            import(&source, dir.path(), &store, &[]),
            Err(ImportError::NotAModel)
        ));
        assert!(store.list_models().unwrap().is_empty());
    }

    #[test]
    fn hashes_files_with_standard_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc");
        std::fs::write(&path, b"abc").unwrap();

        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn known_models_are_pinned_and_distinct() {
        let mut ids = std::collections::HashSet::new();
        let mut files = std::collections::HashSet::new();
        for model in KNOWN_MODELS {
            assert!(ids.insert(model.id), "duplicate id {}", model.id);
            assert!(
                files.insert(model.file_name),
                "duplicate file {}",
                model.file_name
            );
            assert_eq!(model.revision.len(), 40, "{} must pin a commit", model.id);
            assert!(
                model.version.contains(&model.revision[..7]),
                "{}'s version must name its revision",
                model.id
            );
            assert_eq!(model.sha256.len(), 64, "{} needs a sha256", model.id);
            assert!(
                model
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
            assert_eq!(model.min_runtime, RUNTIME_VERSION);
            assert!(
                (1..=MAX_CONTEXT).contains(&model.context_limit),
                "{}",
                model.id
            );
            assert!(!model.description.is_empty());
            assert!(
                model.file_name.contains(model.quantization),
                "{} quantization doesn't match its file",
                model.id
            );
        }
        assert_eq!(KNOWN_MODELS[0], QWEN_3_5_4B);
    }

    #[test]
    fn known_models_are_listed_recommended_first_then_by_size() {
        let keys: Vec<_> = KNOWN_MODELS
            .iter()
            .map(|m| display_order(m.id, m.size_bytes))
            .collect();

        assert!(keys.is_sorted(), "reorder KNOWN_MODELS: {keys:?}");
    }

    #[test]
    fn sorts_installed_models_recommended_first_then_by_size() {
        let model = |id: &str, size_bytes: i64| ModelRecord {
            id: id.into(),
            display_name: id.into(),
            file_path: String::new(),
            size_bytes,
            sha256: String::new(),
            source: ModelSource::Import,
            source_url: None,
            version: None,
            min_runtime: None,
            added_at: 0,
        };
        let mut models = [
            model("local-big", 9_000_000_000),
            model(QWEN_3_5_4B.id, QWEN_3_5_4B.size_bytes as i64),
            model("local-small", 1_000),
        ];

        sort_for_display(&mut models);

        assert_eq!(
            models.map(|m| m.id),
            [QWEN_3_5_4B.id, "local-small", "local-big"]
        );
    }

    #[test]
    fn links_to_the_repository_page() {
        assert_eq!(
            known_by_id("qwen3.5-9b-q4_k_m")
                .map(KnownModel::page_url)
                .as_deref(),
            Some("https://huggingface.co/unsloth/Qwen3.5-9B-GGUF")
        );
        assert_eq!(known_by_id("local-abc123"), None);
    }

    #[test]
    fn context_limit_uses_the_registry_and_falls_back_for_imports() {
        assert_eq!(context_limit(MINISTRAL_3_8B.sha256), 16_384);
        assert_eq!(context_limit(QWEN_3_5_4B.sha256), MAX_CONTEXT);
        assert_eq!(context_limit("0".repeat(64).as_str()), MAX_CONTEXT);
    }

    #[test]
    fn runtime_version_matches_the_llama_dependency() {
        let manifest = std::fs::read_to_string("Cargo.toml").unwrap();

        assert!(
            manifest.contains(&format!(
                "llama-cpp-2 = {{ version = \"={RUNTIME_VERSION}\""
            )),
            "update RUNTIME_VERSION to match Cargo.toml"
        );
    }
}
