//! Audio kept from recordings for testing and review, when the setting is on.
//!
//! Each source's 16 kHz mono stream is saved after resampling and before speech
//! detection, so a file holds everything Whistle could have heard. Files live in
//! `<app data>/recordings/<meeting id>/<offset>-<source>.wav`, where the offset is
//! where that recording starts in the meeting's timeline.
//!
//! The pipeline never waits for the disk: it hands samples to a writer thread
//! through a bounded channel and moves on. If the writer falls behind, the
//! pipeline counts the samples it couldn't hand over, and the writer fills that
//! gap with silence so later audio keeps its time.

use std::fs::{self, File};
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};

use crate::store::Source;
use crate::transcribe::whistle::SAMPLE_RATE;

/// Buffers the writer may fall behind by before audio is dropped from the file
/// (about 10 s at typical 10 ms device buffers).
const BACKLOG: usize = 1024;
/// Samples written between header updates, so a crash loses at most this much.
const FLUSH_EVERY: u64 = 2 * SAMPLE_RATE as u64;

/// The folder that holds every kept recording.
#[derive(Debug, Clone)]
pub struct Recordings {
    root: PathBuf,
}

impl Recordings {
    pub fn new(root: PathBuf) -> Self {
        Recordings { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn meeting_dir(&self, meeting_id: i64) -> PathBuf {
        self.root.join(meeting_id.to_string())
    }

    /// Where one source of one recording is saved.
    pub fn file_path(&self, meeting_id: i64, offset_ms: i64, source: Source) -> PathBuf {
        let side = match source {
            Source::Me => "me",
            Source::Them => "them",
        };
        self.meeting_dir(meeting_id)
            .join(format!("{offset_ms}-{side}.wav"))
    }

    /// Starts saving one source of a recording to a new WAV file.
    pub fn keep(&self, meeting_id: i64, offset_ms: i64, source: Source) -> io::Result<KeptAudio> {
        let path = self.file_path(meeting_id, offset_ms, source);
        fs::create_dir_all(self.meeting_dir(meeting_id))?;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        // `create_new` refuses to replace an earlier recording's file.
        let file = File::options().write(true).create_new(true).open(&path)?;
        let writer = hound::WavWriter::new(BufWriter::new(file), spec).map_err(into_io)?;
        log::info!("keeping {source:?} audio in {}", path.display());
        KeptAudio::spawn(Box::new(Wav { writer, path }))
    }

    /// The total size of every kept recording, in bytes.
    pub fn total_size(&self) -> io::Result<u64> {
        dir_size(&self.root)
    }

    /// Deletes every kept recording.
    pub fn delete_all(&self) -> io::Result<()> {
        remove_dir_if_present(&self.root)
    }

    /// Deletes one meeting's kept recordings.
    pub fn delete_meeting(&self, meeting_id: i64) -> io::Result<()> {
        remove_dir_if_present(&self.meeting_dir(meeting_id))
    }
}

fn remove_dir_if_present(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn dir_size(dir: &Path) -> io::Result<u64> {
    let entries = match fs::read_dir(dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        entries => entries?,
    };
    let mut total = 0;
    for entry in entries {
        let entry = entry?;
        let kind = entry.file_type()?;
        total += if kind.is_dir() {
            dir_size(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
    }
    Ok(total)
}

fn into_io(error: hound::Error) -> io::Error {
    match error {
        hound::Error::IoError(error) => error,
        other => io::Error::other(other),
    }
}

/// Where kept samples go. The writer thread owns it.
pub trait SampleWriter: Send {
    fn write(&mut self, samples: &[f32]) -> io::Result<()>;
    /// Makes what's written so far readable, for example by updating a header.
    fn flush(&mut self) -> io::Result<()>;
    /// Completes the file.
    fn finish(self: Box<Self>) -> io::Result<()>;
}

struct Wav {
    writer: hound::WavWriter<BufWriter<File>>,
    path: PathBuf,
}

impl SampleWriter for Wav {
    fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        for &sample in samples {
            let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            self.writer.write_sample(value).map_err(into_io)?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush().map_err(into_io)
    }

    fn finish(self: Box<Self>) -> io::Result<()> {
        let Wav { writer, path } = *self;
        writer.finalize().map_err(into_io)?;
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        log::info!("saved {size} bytes of audio to {}", path.display());
        Ok(())
    }
}

struct Chunk {
    /// Samples the pipeline couldn't hand over before these, written as silence.
    gap: u64,
    samples: Vec<f32>,
}

/// One source's kept audio. The pipeline pushes samples without waiting, and
/// `finish` completes the file once the recording ends.
pub struct KeptAudio {
    chunks: Option<SyncSender<Chunk>>,
    writer: Option<JoinHandle<()>>,
    /// Samples not yet handed over, owed to the writer as silence.
    gap: u64,
    dropped: u64,
}

impl KeptAudio {
    /// Starts a writer thread that saves pushed samples to `writer`.
    pub fn spawn(writer: Box<dyn SampleWriter>) -> io::Result<Self> {
        let (chunks, received) = sync_channel(BACKLOG);
        let writer = thread::Builder::new()
            .name("kept-audio-writer".into())
            .spawn(move || write_all(received, writer))?;
        Ok(KeptAudio {
            chunks: Some(chunks),
            writer: Some(writer),
            gap: 0,
            dropped: 0,
        })
    }

    /// Hands samples to the writer. Never blocks: when the writer is behind,
    /// the samples are dropped and later replaced with silence.
    pub fn push(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        let Some(chunks) = &self.chunks else {
            return;
        };
        let chunk = Chunk {
            gap: self.gap,
            samples: samples.to_vec(),
        };
        match chunks.try_send(chunk) {
            Ok(()) => self.gap = 0,
            Err(TrySendError::Full(chunk)) => {
                self.gap += chunk.samples.len() as u64;
                self.dropped += chunk.samples.len() as u64;
            }
            // The writer stopped after an error it already logged.
            Err(TrySendError::Disconnected(_)) => self.chunks = None,
        }
    }

    /// Completes the file and waits for the writer to finish.
    pub fn finish(mut self) {
        self.close();
    }

    fn close(&mut self) {
        // Blocking is fine now that the recording is over: hand over any silence still owed.
        if let Some(chunks) = self.chunks.take()
            && self.gap > 0
        {
            let _ = chunks.send(Chunk {
                gap: self.gap,
                samples: Vec::new(),
            });
        }
        if let Some(writer) = self.writer.take()
            && writer.join().is_err()
        {
            log::error!("the kept audio writer panicked");
        }
        if self.dropped > 0 {
            log::warn!(
                "kept audio is missing {} ms because the disk fell behind",
                self.dropped * 1000 / SAMPLE_RATE as u64
            );
        }
    }
}

impl Drop for KeptAudio {
    fn drop(&mut self) {
        self.close();
    }
}

fn write_all(chunks: Receiver<Chunk>, mut writer: Box<dyn SampleWriter>) {
    let mut unflushed = 0;
    let result = (|| -> io::Result<()> {
        for chunk in chunks {
            if chunk.gap > 0 {
                writer.write(&vec![0.0; chunk.gap as usize])?;
            }
            writer.write(&chunk.samples)?;
            unflushed += chunk.gap + chunk.samples.len() as u64;
            if unflushed >= FLUSH_EVERY {
                writer.flush()?;
                unflushed = 0;
            }
        }
        Ok(())
    })();
    let result = result.and_then(|()| writer.finish());
    if let Err(error) = result {
        log::error!("couldn't save kept audio: {error}");
    }
}

/// A writer for tests that waits until released.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Records what it's given, and waits for `release` before its first write.
    struct BlockedWriter {
        release: Option<Receiver<()>>,
        written: Written,
    }

    impl SampleWriter for BlockedWriter {
        fn write(&mut self, samples: &[f32]) -> io::Result<()> {
            if let Some(release) = self.release.take() {
                let _ = release.recv();
            }
            self.written.lock().unwrap().extend_from_slice(samples);
            Ok(())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
        fn finish(self: Box<Self>) -> io::Result<()> {
            Ok(())
        }
    }

    /// Everything a `BlockedWriter` has written.
    pub type Written = Arc<Mutex<Vec<f32>>>;

    /// A writer that waits until released, and the handles to release it and see what it wrote.
    pub fn blocked_writer() -> (Box<dyn SampleWriter>, SyncSender<()>, Written) {
        let (release, released) = sync_channel(1);
        let written = Arc::new(Mutex::new(Vec::new()));
        let writer = BlockedWriter {
            release: Some(released),
            written: written.clone(),
        };
        (Box::new(writer), release, written)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::blocked_writer;
    use super::*;

    fn read_wav(path: &Path) -> (hound::WavSpec, Vec<i16>) {
        let mut reader = hound::WavReader::open(path).unwrap();
        let samples = reader.samples::<i16>().map(Result::unwrap).collect();
        (reader.spec(), samples)
    }

    #[test]
    fn saves_16k_mono_wav_named_by_offset_and_source() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = Recordings::new(dir.path().join("recordings"));

        let mut kept = recordings.keep(3, 1500, Source::Them).unwrap();
        kept.push(&[0.5; 1600]);
        kept.push(&[-1.0; 1600]);
        kept.finish();

        let path = dir.path().join("recordings/3/1500-them.wav");
        assert_eq!(recordings.file_path(3, 1500, Source::Them), path);
        let (spec, samples) = read_wav(&path);
        assert_eq!(
            (spec.channels, spec.sample_rate, spec.bits_per_sample),
            (1, 16_000, 16)
        );
        assert_eq!(samples.len(), 3200);
        assert_eq!(samples[0], 16_384);
        assert_eq!(samples[3199], -i16::MAX);
    }

    #[test]
    fn never_replaces_an_earlier_recording() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = Recordings::new(dir.path().to_path_buf());
        recordings.keep(1, 0, Source::Me).unwrap().finish();

        let error = recordings.keep(1, 0, Source::Me).err().unwrap();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn a_crash_leaves_a_readable_file() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = Recordings::new(dir.path().to_path_buf());
        let mut kept = recordings.keep(1, 0, Source::Me).unwrap();
        kept.push(&vec![0.25; FLUSH_EVERY as usize]);
        // Without finishing, as after a crash, the flushed header still describes the samples.
        let path = recordings.file_path(1, 0, Source::Me);
        let flushed = || {
            hound::WavReader::open(&path)
                .map(|reader| reader.len() as u64 >= FLUSH_EVERY)
                .unwrap_or(false)
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !flushed() {
            assert!(std::time::Instant::now() < deadline, "never flushed");
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(read_wav(&path).1.len(), FLUSH_EVERY as usize);
        kept.finish();
    }

    #[test]
    fn sizes_and_deletes_recordings() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = Recordings::new(dir.path().join("recordings"));
        assert_eq!(recordings.total_size().unwrap(), 0);
        for (meeting, source) in [(1, Source::Me), (1, Source::Them), (2, Source::Me)] {
            let mut kept = recordings.keep(meeting, 0, source).unwrap();
            kept.push(&[0.1; 16_000]);
            kept.finish();
        }
        // Each file is a 44-byte header and 2 bytes per sample.
        assert_eq!(recordings.total_size().unwrap(), 3 * (44 + 32_000));

        recordings.delete_meeting(1).unwrap();
        assert!(!recordings.file_path(1, 0, Source::Me).exists());
        assert!(recordings.file_path(2, 0, Source::Me).exists());

        recordings.delete_all().unwrap();
        assert_eq!(recordings.total_size().unwrap(), 0);
        // Deleting what's already gone is fine.
        recordings.delete_all().unwrap();
        recordings.delete_meeting(9).unwrap();
    }

    #[test]
    fn a_slow_writer_never_blocks_and_keeps_time_with_silence() {
        let (writer, release, written) = blocked_writer();
        let mut kept = KeptAudio::spawn(writer).unwrap();

        // Far more buffers than the backlog holds; none of these calls may wait.
        let pushes = BACKLOG + 500;
        for _ in 0..pushes {
            kept.push(&[0.25; 160]);
        }
        release.send(()).unwrap();
        kept.finish();

        let written = written.lock().unwrap();
        // Dropped buffers come back as silence, so the length matches what was pushed.
        assert_eq!(written.len(), pushes * 160);
        assert_eq!(written[0], 0.25);
        assert!(written.contains(&0.0));
    }
}
