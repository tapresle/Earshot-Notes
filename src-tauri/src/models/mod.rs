//! Local LLM files: the recommended models and importing a GGUF from disk.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::store::{ModelRecord, ModelSource, Store, StoreError};

/// The llama-cpp-2 version this build runs, recorded as the minimum runtime of
/// models known to work with it. Must match Cargo.toml.
pub const RUNTIME_VERSION: &str = "0.1.158";

/// A model the app knows how to fetch and verify.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownModel {
    pub id: &'static str,
    pub display_name: &'static str,
    pub file_name: &'static str,
    pub repo: &'static str,
    pub revision: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
    pub version: &'static str,
    pub min_runtime: &'static str,
}

impl KnownModel {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.revision, self.file_name
        )
    }
}

/// The recommended summary model.
pub const GEMMA_4_E2B: KnownModel = KnownModel {
    id: "gemma-4-e2b-it-q4_k_m",
    display_name: "Gemma 4 E2B",
    file_name: "gemma-4-E2B-it-Q4_K_M.gguf",
    repo: "unsloth/gemma-4-E2B-it-GGUF",
    revision: "0314792d7f1f7e229411f620751375812bb9faf2",
    sha256: "740185b21d22ceb83a11c3aa62ad5842ef32c70f6096d756bbee85a1e4ec34b8",
    size_bytes: 3_106_738_272,
    version: "gemma-4-E2B-it Q4_K_M (unsloth 0314792)",
    min_runtime: "0.1.158",
};

pub const KNOWN_MODELS: &[KnownModel] = &[GEMMA_4_E2B];

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("that file isn't a GGUF model")]
    NotAModel,
    #[error("couldn't read or copy the model: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Copies a GGUF file into `models_dir`, records it, and selects it for
/// summaries. Importing a file that's already installed returns the existing record.
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
        store.select_summary_model(&existing.id)?;
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
    let saved = store.add_model(&record).and_then(|saved| {
        store.select_summary_model(&saved.id)?;
        Ok(saved)
    });
    if saved.is_err() {
        let _ = std::fs::remove_file(&dest);
    }
    Ok(saved?)
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
    fn imports_copies_and_selects_a_model() {
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
        assert_eq!(store.summary_model().unwrap(), Some(record));
    }

    #[test]
    fn recognizes_a_known_model_by_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let source = fake_gguf(dir.path(), "downloaded.gguf", b"known");
        let sha256: &'static str = Box::leak(sha256_file(&source).unwrap().into_boxed_str());
        let known = KnownModel {
            sha256,
            ..GEMMA_4_E2B
        };

        let record = import(&source, &dir.path().join("models"), &store, &[known]).unwrap();

        assert_eq!(record.id, GEMMA_4_E2B.id);
        assert_eq!(record.display_name, "Gemma 4 E2B");
        assert!(record.file_path.ends_with(GEMMA_4_E2B.file_name));
        assert_eq!(record.version.as_deref(), Some(GEMMA_4_E2B.version));
        assert_eq!(record.min_runtime.as_deref(), Some("0.1.158"));
        assert_eq!(record.source_url, Some(GEMMA_4_E2B.url()));
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
    fn runtime_version_matches_the_llama_dependency() {
        let manifest = std::fs::read_to_string("Cargo.toml").unwrap();

        assert!(
            manifest.contains(&format!("llama-cpp-2 = {{ version = \"{RUNTIME_VERSION}\"")),
            "update RUNTIME_VERSION to match Cargo.toml"
        );
    }
}
