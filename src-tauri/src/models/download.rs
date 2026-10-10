//! Downloads a known model with resume, progress, and checksum verification.
//! This is the app's only network code, and it runs only when the user asks.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::{KnownModel, sha256_file};
use crate::store::{ModelRecord, ModelSource, Store, StoreError};

/// An open HTTP response body.
pub struct Fetched {
    pub body: Box<dyn Read + Send>,
    /// True when the server honored the range request and the body continues
    /// from the requested offset.
    pub resumed: bool,
}

/// Fetches a URL starting at a byte offset. Abstracted so tests can serve
/// bytes from memory.
pub trait Fetch: Send + Sync {
    fn fetch(&self, url: &str, offset: u64) -> io::Result<Fetched>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "phase")]
pub enum Phase {
    Downloading { received: u64, total: u64 },
    Verifying,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("the download was cancelled")]
    Cancelled,
    #[error("there isn't enough free disk space for this model")]
    DiskFull,
    #[error("the downloaded file didn't match its checksum, so it was deleted. Try again")]
    Corrupt,
    #[error("the download failed: {0}")]
    Network(String),
    #[error("couldn't save the model: {0}")]
    Io(io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<io::Error> for DownloadError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::StorageFull {
            DownloadError::DiskFull
        } else {
            DownloadError::Io(error)
        }
    }
}

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// How often the download checks for cancellation while it waits for data.
const CANCEL_POLL: Duration = Duration::from_millis(200);

/// How long the server may send nothing before the download fails as a
/// network error. The partial file is kept, so retrying resumes. ureq has no
/// per-read timeout (`timeout_recv_body` bounds the whole body, which would
/// cut off large models on slow connections), so stalls are detected here.
const STALL_LIMIT: Duration = Duration::from_secs(60);

/// Downloads a summary model into `models_dir` with [`fetch_verified`] and
/// records it. It becomes the summary model only when no other model is installed.
pub fn download(
    model: &KnownModel,
    models_dir: &Path,
    store: &Store,
    fetch: &dyn Fetch,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Phase),
) -> Result<ModelRecord, DownloadError> {
    if let Some(existing) = store.model_by_sha256(model.sha256)? {
        return Ok(existing);
    }
    let dest = fetch_verified(model, models_dir, fetch, cancel, on_progress)?;

    Ok(store.add_model_keeping_selection(&ModelRecord {
        id: model.id.to_owned(),
        display_name: model.display_name.to_owned(),
        file_path: dest.to_string_lossy().into_owned(),
        size_bytes: model.size_bytes as i64,
        sha256: model.sha256.to_owned(),
        source: ModelSource::Download,
        source_url: Some(model.url()),
        version: Some(model.version.to_owned()),
        min_runtime: Some(model.min_runtime.to_owned()),
        added_at: 0,
    })?)
}

/// Downloads `model` to `models_dir/<file name>` and verifies its size and
/// checksum, returning the file's path. A partial download from an earlier
/// attempt is resumed. The partial file is kept on cancellation or network
/// failure so the next attempt resumes, and deleted if the finished file fails
/// verification.
pub fn fetch_verified(
    model: &KnownModel,
    models_dir: &Path,
    fetch: &dyn Fetch,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Phase),
) -> Result<PathBuf, DownloadError> {
    std::fs::create_dir_all(models_dir)?;
    let dest = models_dir.join(model.file_name);
    let part = partial_path(&dest);

    let offset = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if offset < model.size_bytes {
        fetch_into(
            model,
            &part,
            offset,
            fetch,
            cancel,
            STALL_LIMIT,
            on_progress,
        )?;
    }

    on_progress(Phase::Verifying);
    let size = std::fs::metadata(&part)?.len();
    if size != model.size_bytes || sha256_file(&part)? != model.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(DownloadError::Corrupt);
    }
    std::fs::rename(&part, &dest)?;
    Ok(dest)
}

/// Where an in-progress download is written.
pub fn partial_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

/// Writes the response body into `part`. The body is read on its own thread so
/// that a read blocked on a stalled connection can't delay cancellation: this
/// loop wakes every `CANCEL_POLL` to check `cancel`, and fails once nothing has
/// arrived for `stall_limit`. Either way the reader thread is left to finish
/// its blocked read, and it exits when the read returns because nothing
/// receives its data any more. Only this thread writes to `part`, so a resumed
/// download can start right away.
fn fetch_into(
    model: &KnownModel,
    part: &Path,
    offset: u64,
    fetch: &dyn Fetch,
    cancel: &AtomicBool,
    stall_limit: Duration,
    on_progress: &mut dyn FnMut(Phase),
) -> Result<(), DownloadError> {
    let fetched = fetch
        .fetch(&model.url(), offset)
        .map_err(|e| DownloadError::Network(e.to_string()))?;
    // A server that ignores the range sends the whole file again.
    let (mut file, mut received) = if fetched.resumed && offset > 0 {
        (OpenOptions::new().append(true).open(part)?, offset)
    } else {
        (File::create(part)?, 0)
    };
    let chunks = read_in_background(fetched.body)?;
    let mut last_report = Instant::now() - PROGRESS_INTERVAL;
    let mut last_data = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            file.flush()?;
            return Err(DownloadError::Cancelled);
        }
        let chunk = match chunks.recv_timeout(CANCEL_POLL) {
            Ok(chunk) => chunk.map_err(|e| DownloadError::Network(e.to_string()))?,
            Err(RecvTimeoutError::Timeout) => {
                if last_data.elapsed() >= stall_limit {
                    file.flush()?;
                    return Err(DownloadError::Network(format!(
                        "the server sent nothing for {} seconds",
                        stall_limit.as_secs()
                    )));
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(DownloadError::Network(
                    "the download stopped unexpectedly".into(),
                ));
            }
        };
        if chunk.is_empty() {
            break;
        }
        last_data = Instant::now();
        file.write_all(&chunk)?;
        received += chunk.len() as u64;
        if last_report.elapsed() >= PROGRESS_INTERVAL {
            on_progress(Phase::Downloading {
                received,
                total: model.size_bytes,
            });
            last_report = Instant::now();
        }
    }
    file.flush()?;
    on_progress(Phase::Downloading {
        received,
        total: model.size_bytes,
    });
    Ok(())
}

/// Reads `body` on a new thread and sends each chunk, ending with an empty
/// chunk at the end of the body or with the first error. The thread stops
/// after its current read once the receiver is dropped.
fn read_in_background(mut body: Box<dyn Read + Send>) -> io::Result<Receiver<io::Result<Vec<u8>>>> {
    let (sender, receiver) = mpsc::sync_channel(4);
    std::thread::Builder::new()
        .name("model-download".into())
        .spawn(move || {
            let mut buffer = vec![0u8; 1 << 20];
            loop {
                let chunk = match body.read(&mut buffer) {
                    Ok(read) => Ok(buffer[..read].to_vec()),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => Err(error),
                };
                let more = matches!(&chunk, Ok(bytes) if !bytes.is_empty());
                if sender.send(chunk).is_err() || !more {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

/// Fetches over HTTPS with ureq, following Hugging Face's redirect to its CDN.
pub struct HttpFetch {
    agent: ureq::Agent,
}

impl Default for HttpFetch {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Fetch for HttpFetch {
    fn fetch(&self, url: &str, offset: u64) -> io::Result<Fetched> {
        let mut request = self.agent.get(url);
        if offset > 0 {
            request = request.header("Range", format!("bytes={offset}-"));
        }
        let response = request.call().map_err(io::Error::other)?;
        let status = response.status().as_u16();
        if !(status == 200 || status == 206) {
            return Err(io::Error::other(format!(
                "the server answered HTTP {status}"
            )));
        }
        Ok(Fetched {
            resumed: status == 206,
            body: Box::new(response.into_body().into_reader()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serves `data` from memory. Optionally ignores ranges, or fails after
    /// sending `fail_after` bytes.
    struct FakeServer {
        data: Vec<u8>,
        honor_range: bool,
        fail_after: Option<usize>,
        requests: Mutex<Vec<u64>>,
    }

    impl FakeServer {
        fn new(data: &[u8]) -> Self {
            Self {
                data: data.to_vec(),
                honor_range: true,
                fail_after: None,
                requests: Mutex::default(),
            }
        }
    }

    struct FailingReader {
        inner: io::Cursor<Vec<u8>>,
        remaining: usize,
    }

    impl Read for FailingReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "connection reset",
                ));
            }
            let limit = buf.len().min(self.remaining);
            let read = self.inner.read(&mut buf[..limit])?;
            self.remaining -= read;
            Ok(read)
        }
    }

    impl Fetch for FakeServer {
        fn fetch(&self, _url: &str, offset: u64) -> io::Result<Fetched> {
            self.requests.lock().unwrap().push(offset);
            let start = if self.honor_range { offset as usize } else { 0 };
            let bytes = self.data[start..].to_vec();
            let body: Box<dyn Read + Send> = match self.fail_after {
                Some(n) => Box::new(FailingReader {
                    inner: io::Cursor::new(bytes),
                    remaining: n,
                }),
                None => Box::new(io::Cursor::new(bytes)),
            };
            Ok(Fetched {
                body,
                resumed: self.honor_range && offset > 0,
            })
        }
    }

    fn model_for(data: &[u8]) -> KnownModel {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, data).unwrap();
        KnownModel {
            id: "test-model",
            display_name: "Test model",
            file_name: "test.gguf",
            quantization: "Q4_K_M",
            repo: "example/test",
            revision: "abc",
            sha256: Box::leak(sha256_file(&path).unwrap().into_boxed_str()),
            size_bytes: data.len() as u64,
            version: "test v1",
            min_runtime: "0.1.158",
            description: "Test model.",
            context_limit: 4096,
        }
    }

    fn data(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    fn run(
        model: &KnownModel,
        dir: &Path,
        store: &Store,
        server: &FakeServer,
    ) -> (Result<ModelRecord, DownloadError>, Vec<Phase>) {
        let mut phases = Vec::new();
        let result = download(
            model,
            dir,
            store,
            server,
            &AtomicBool::new(false),
            &mut |p| phases.push(p),
        );
        (result, phases)
    }

    #[test]
    fn downloads_verifies_records_and_selects_a_first_model() {
        let bytes = data(3_000_000);
        let model = model_for(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();

        let (result, phases) = run(&model, dir.path(), &store, &FakeServer::new(&bytes));

        let record = result.unwrap();
        assert_eq!(std::fs::read(&record.file_path).unwrap(), bytes);
        assert_eq!(record.source, ModelSource::Download);
        assert_eq!(record.version.as_deref(), Some("test v1"));
        assert_eq!(record.source_url, Some(model.url()));
        assert_eq!(store.summary_model().unwrap(), Some(record));
        assert!(!partial_path(&dir.path().join("test.gguf")).exists());
        assert_eq!(
            phases[phases.len() - 2..],
            [
                Phase::Downloading {
                    received: 3_000_000,
                    total: 3_000_000
                },
                Phase::Verifying
            ]
        );
    }

    fn installed(store: &Store, id: &str) -> ModelRecord {
        store
            .add_model(&ModelRecord {
                id: id.into(),
                display_name: id.into(),
                file_path: format!("/models/{id}.gguf"),
                size_bytes: 1,
                sha256: id.repeat(64)[..64].into(),
                source: ModelSource::Import,
                source_url: None,
                version: None,
                min_runtime: None,
                added_at: 0,
            })
            .unwrap()
    }

    #[test]
    fn keeps_the_selected_model_when_another_downloads() {
        let bytes = data(1_000_000);
        let model = model_for(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        installed(&store, "a");
        let selected = installed(&store, "b");
        store.select_summary_model(&selected.id).unwrap();

        run(&model, dir.path(), &store, &FakeServer::new(&bytes))
            .0
            .unwrap();

        assert_eq!(store.summary_model().unwrap(), Some(selected));
    }

    #[test]
    fn keeps_the_only_model_in_use_even_without_an_explicit_selection() {
        let bytes = data(1_000_000);
        let model = model_for(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let existing = installed(&store, "a");

        run(&model, dir.path(), &store, &FakeServer::new(&bytes))
            .0
            .unwrap();

        assert_eq!(store.summary_model().unwrap(), Some(existing));
    }

    #[test]
    fn keeps_the_partial_file_after_a_network_failure_and_resumes() {
        let bytes = data(3_000_000);
        let model = model_for(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let mut flaky = FakeServer::new(&bytes);
        flaky.fail_after = Some(1_200_000);

        let (first, _) = run(&model, dir.path(), &store, &flaky);
        assert!(matches!(first, Err(DownloadError::Network(_))), "{first:?}");
        let part = partial_path(&dir.path().join("test.gguf"));
        assert_eq!(std::fs::metadata(&part).unwrap().len(), 1_200_000);

        let server = FakeServer::new(&bytes);
        let (second, _) = run(&model, dir.path(), &store, &server);

        assert_eq!(std::fs::read(second.unwrap().file_path).unwrap(), bytes);
        assert_eq!(*server.requests.lock().unwrap(), vec![1_200_000]);
    }

    #[test]
    fn starts_over_when_the_server_ignores_the_range() {
        let bytes = data(2_000_000);
        let model = model_for(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(
            partial_path(&dir.path().join("test.gguf")),
            &bytes[..500_000],
        )
        .unwrap();
        let mut server = FakeServer::new(&bytes);
        server.honor_range = false;

        let (result, _) = run(&model, dir.path(), &store, &server);

        assert_eq!(std::fs::read(result.unwrap().file_path).unwrap(), bytes);
    }

    #[test]
    fn deletes_a_download_that_fails_verification() {
        let bytes = data(1_000_000);
        let model = model_for(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let mut tampered = bytes.clone();
        tampered[500] ^= 0xFF;

        let (result, phases) = run(&model, dir.path(), &store, &FakeServer::new(&tampered));

        assert!(matches!(result, Err(DownloadError::Corrupt)));
        assert_eq!(phases.last(), Some(&Phase::Verifying));
        assert!(!partial_path(&dir.path().join("test.gguf")).exists());
        assert!(!dir.path().join("test.gguf").exists());
        assert!(store.list_models().unwrap().is_empty());
    }

    #[test]
    fn stops_when_cancelled_and_keeps_the_partial_file() {
        let bytes = data(3_000_000);
        let model = model_for(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let cancel = AtomicBool::new(false);

        let result = download(
            &model,
            dir.path(),
            &store,
            &FakeServer::new(&bytes),
            &cancel,
            &mut |phase| {
                if matches!(phase, Phase::Downloading { received, .. } if received > 0) {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );

        assert!(matches!(result, Err(DownloadError::Cancelled)));
        assert!(partial_path(&dir.path().join("test.gguf")).exists());
        assert!(store.list_models().unwrap().is_empty());
    }

    /// Sends every request to a local test server instead of the model's URL.
    struct LocalHttp(String);

    impl Fetch for LocalHttp {
        fn fetch(&self, _url: &str, offset: u64) -> io::Result<Fetched> {
            HttpFetch::default().fetch(&self.0, offset)
        }
    }

    #[test]
    fn cancel_stops_a_stalled_http_download_and_keeps_the_partial_file() {
        let bytes = data(3_000_000);
        let model = model_for(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let part = partial_path(&dir.path().join("test.gguf"));
        let sent = 200_000;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let fetch = LocalHttp(format!(
            "http://{}/test.gguf",
            listener.local_addr().unwrap()
        ));
        let (release, released) = mpsc::channel::<()>();
        // Sends the headers and the first `sent` bytes, then stalls with the
        // connection open until the test is done, or 10 s as a backstop.
        let server = std::thread::spawn({
            let bytes = bytes.clone();
            move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                    bytes.len()
                )
                .unwrap();
                stream.write_all(&bytes[..sent]).unwrap();
                stream.flush().unwrap();
                let _ = released.recv_timeout(Duration::from_secs(10));
            }
        });
        let cancel = AtomicBool::new(false);

        let (result, cancel_took) = std::thread::scope(|scope| {
            let canceller = scope.spawn(|| {
                let deadline = Instant::now() + Duration::from_secs(10);
                while std::fs::metadata(&part).map_or(0, |m| m.len()) < sent as u64 {
                    assert!(Instant::now() < deadline, "the first bytes never arrived");
                    std::thread::sleep(Duration::from_millis(10));
                }
                // Let the download block on the stalled connection.
                std::thread::sleep(Duration::from_millis(500));
                cancel.store(true, Ordering::Relaxed);
                Instant::now()
            });
            let result = download(&model, dir.path(), &store, &fetch, &cancel, &mut |_| {});
            let finished = Instant::now();
            (result, finished - canceller.join().unwrap())
        });
        drop(release);
        server.join().unwrap();

        assert!(
            matches!(result, Err(DownloadError::Cancelled)),
            "{result:?}"
        );
        assert!(
            cancel_took < Duration::from_secs(2),
            "cancelling took {cancel_took:?}"
        );
        assert_eq!(std::fs::metadata(&part).unwrap().len(), sent as u64);
        assert!(store.list_models().unwrap().is_empty());
    }

    /// Sends `data`, then blocks until `release` is dropped, like a connection
    /// that stops sending.
    struct StallingReader {
        data: io::Cursor<Vec<u8>>,
        release: Receiver<()>,
    }

    impl Read for StallingReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let read = self.data.read(buf)?;
            if read > 0 {
                return Ok(read);
            }
            let _ = self.release.recv();
            Err(io::ErrorKind::ConnectionAborted.into())
        }
    }

    struct StallingServer(Mutex<Option<StallingReader>>);

    impl Fetch for StallingServer {
        fn fetch(&self, _url: &str, _offset: u64) -> io::Result<Fetched> {
            Ok(Fetched {
                body: Box::new(self.0.lock().unwrap().take().unwrap()),
                resumed: false,
            })
        }
    }

    #[test]
    fn fails_when_the_server_stops_sending_and_keeps_the_partial_file() {
        let bytes = data(1_000_000);
        let model = model_for(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let part = partial_path(&dir.path().join("test.gguf"));
        let (release, released) = mpsc::channel();
        let server = StallingServer(Mutex::new(Some(StallingReader {
            data: io::Cursor::new(bytes[..300_000].to_vec()),
            release: released,
        })));

        let started = Instant::now();
        let result = fetch_into(
            &model,
            &part,
            0,
            &server,
            &AtomicBool::new(false),
            Duration::from_millis(500),
            &mut |_| {},
        );
        let took = started.elapsed();
        drop(release);

        assert!(
            matches!(&result, Err(DownloadError::Network(message)) if message.contains("sent nothing")),
            "{result:?}"
        );
        assert!(took < Duration::from_secs(2), "giving up took {took:?}");
        assert_eq!(std::fs::metadata(&part).unwrap().len(), 300_000);
    }

    #[test]
    fn reuses_a_model_that_is_already_installed() {
        let bytes = data(1000);
        let model = model_for(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        run(&model, dir.path(), &store, &FakeServer::new(&bytes))
            .0
            .unwrap();
        let server = FakeServer::new(&bytes);

        let (again, _) = run(&model, dir.path(), &store, &server);

        assert!(again.is_ok());
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn reports_a_full_disk_in_plain_words() {
        let error = DownloadError::from(io::Error::from(io::ErrorKind::StorageFull));

        assert_eq!(
            error.to_string(),
            "there isn't enough free disk space for this model"
        );
    }

    /// Talks to the real Hugging Face CDN, so it's ignored by default. Run it
    /// with `cargo test --lib -- --ignored resumes_from_hugging_face`. It fetches
    /// only the last kilobyte of the test model fetched by scripts/fetch-native.sh.
    #[test]
    #[ignore = "needs the network"]
    fn resumes_from_hugging_face() {
        let local = std::fs::read("tests/models/smollm2-135m-instruct-q4_k_m.gguf").unwrap();
        let offset = local.len() as u64 - 1024;
        let url = "https://huggingface.co/bartowski/SmolLM2-135M-Instruct-GGUF/resolve/09816acd5d99df7be770d85ea30822623dab342c/SmolLM2-135M-Instruct-Q4_K_M.gguf";

        let fetched = HttpFetch::default().fetch(url, offset).unwrap();
        let mut tail = Vec::new();
        fetched.body.take(4096).read_to_end(&mut tail).unwrap();

        assert!(
            fetched.resumed,
            "the CDN should honor the range after redirecting"
        );
        assert_eq!(tail, local[offset as usize..]);
    }
}
