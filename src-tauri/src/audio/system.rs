//! Capture of everything the system plays (the other side of a call), through
//! a Core Audio process tap on macOS.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use macos_audio::{Format, OutputDeviceWatcher, Permission, SystemAudioTap};

use super::{Buffer, Capture, CaptureError, CaptureFormat, ErrorSink};

/// How long to let a device switch settle before rebuilding, so a burst of
/// notifications (AirPods often send several) rebuilds once.
const SETTLE: Duration = Duration::from_millis(300);
/// The longest gap filled with silence after a rebuild. Longer outages, such as
/// a rebuild that failed and succeeded on a later change, shift later times.
const MAX_GAP: Duration = Duration::from_secs(10);

/// Receives interleaved samples and the rate they were captured at, on a real-time thread.
type OnSamples = Box<dyn FnMut(&[f32], u32) + Send>;
/// Called on a background thread when the default output device changes. Must not block.
type OnChange = Box<dyn Fn() + Send + Sync>;

/// Starts a tap on the current default output device. A trait so tests can
/// stand in for Core Audio.
trait TapStarter: Send + 'static {
    /// Starts a tap; dropping the returned value stops it.
    fn start(&mut self, on_samples: OnSamples) -> Result<(Box<dyn Send>, Format), String>;
}

struct CoreAudioTaps;

impl TapStarter for CoreAudioTaps {
    fn start(&mut self, on_samples: OnSamples) -> Result<(Box<dyn Send>, Format), String> {
        SystemAudioTap::start(on_samples)
            .map(|(tap, format)| (Box::new(tap) as Box<dyn Send>, format))
    }
}

/// Starts watching for output device changes, calling the given function on
/// each one. Dropping the returned value stops watching.
type WatchStarter = Box<dyn FnOnce(OnChange) -> Result<Box<dyn Send>, String>>;

fn watch_core_audio() -> WatchStarter {
    Box::new(|on_change| {
        OutputDeviceWatcher::start(on_change).map(|w| Box::new(w) as Box<dyn Send>)
    })
}

pub struct SystemCapture {
    // Dropped first, so no change arrives while the rest shuts down.
    watcher: Option<Box<dyn Send>>,
    changes: Option<SyncSender<()>>,
    follower: Option<JoinHandle<()>>,
    dropped: Arc<AtomicU64>,
}

impl Capture for SystemCapture {}

impl Drop for SystemCapture {
    fn drop(&mut self) {
        drop(self.watcher.take());
        // Ends the follower thread, which stops the tap and drops the callback,
        // closing the sender.
        drop(self.changes.take());
        if let Some(follower) = self.follower.take()
            && follower.join().is_err()
        {
            log::error!("the system audio follower thread panicked");
        }
        let dropped = self.dropped.load(Ordering::Relaxed);
        if dropped > 0 {
            log::warn!(
                "system audio capture dropped {dropped} buffers because transcription fell behind"
            );
        }
    }
}

/// Starts capturing system audio, asking for permission first if needed (this
/// blocks until the prompt is answered). When macOS won't report the
/// permission, capture starts anyway; a denied permission then records silence.
/// When the default output device changes, capture moves to the new device.
pub fn start(
    frames: SyncSender<Buffer>,
    on_error: ErrorSink,
) -> Result<(SystemCapture, CaptureFormat), CaptureError> {
    check_access(
        macos_audio::system_audio_permission(),
        macos_audio::request_system_audio_access,
    )?;
    start_following(CoreAudioTaps, watch_core_audio(), frames, on_error)
}

/// Starts a tap through `taps`, and rebuilds it through `taps` whenever `watch`
/// reports that the output device changed.
fn start_following(
    taps: impl TapStarter,
    watch: WatchStarter,
    frames: SyncSender<Buffer>,
    on_error: ErrorSink,
) -> Result<(SystemCapture, CaptureFormat), CaptureError> {
    let mut follower = Follower::new(Box::new(taps), frames, on_error);
    let dropped = follower.dropped.clone();
    let format = follower
        .start_tap(None)
        .map_err(CaptureError::SystemAudio)?;
    log::info!(
        "system audio capture started on \"{}\" at {} Hz, {} channels (tap format {} Hz)",
        format.device_name,
        format.sample_rate,
        format.channels,
        format.tap_sample_rate
    );

    // One pending signal is enough: a rebuild always uses the device current then.
    let (changes, changed) = sync_channel(1);
    let notify = changes.clone();
    // Without the watcher, capture still works but stays on this device.
    let watcher = watch(Box::new(move || {
        let _ = notify.try_send(());
    }))
    .inspect_err(|error| log::warn!("system audio won't follow output device changes: {error}"))
    .ok();
    let follower = thread::Builder::new()
        .name("system-audio-follower".into())
        .spawn(move || follower.follow(changed))
        .map_err(|e| CaptureError::SystemAudio(e.to_string()))?;

    Ok((
        SystemCapture {
            watcher,
            changes: Some(changes),
            follower: Some(follower),
            dropped,
        },
        CaptureFormat {
            sample_rate: format.sample_rate,
            channels: format.channels,
        },
    ))
}

/// Owns the running tap and replaces it when the output device changes.
struct Follower {
    taps: Box<dyn TapStarter>,
    tap: Option<Box<dyn Send>>,
    frames: SyncSender<Buffer>,
    dropped: Arc<AtomicU64>,
    on_error: ErrorSink,
    /// The interleaved layout the pipeline downmixes with, fixed by the first tap.
    channels: u16,
    /// When the last buffer arrived, in nanoseconds after `epoch`.
    last_buffer: Arc<AtomicU64>,
    epoch: Instant,
}

impl Follower {
    fn new(taps: Box<dyn TapStarter>, frames: SyncSender<Buffer>, on_error: ErrorSink) -> Self {
        Follower {
            taps,
            tap: None,
            frames,
            dropped: Arc::new(AtomicU64::new(0)),
            on_error,
            channels: 0,
            last_buffer: Arc::new(AtomicU64::new(0)),
            epoch: Instant::now(),
        }
    }

    /// Rebuilds the tap after each change until `changed` closes, then stops it.
    fn follow(mut self, changed: Receiver<()>) {
        while changed.recv().is_ok() {
            thread::sleep(SETTLE);
            while changed.try_recv().is_ok() {}
            self.device_changed();
        }
    }

    fn device_changed(&mut self) {
        // Stop the old tap first, so its last buffers can't interleave with the new one's.
        drop(self.tap.take());
        let last = self.epoch + Duration::from_nanos(self.last_buffer.load(Ordering::Relaxed));
        let result = self.start_tap(Some(last)).and_then(|format| {
            if format.channels == self.channels {
                Ok(format)
            } else {
                drop(self.tap.take());
                Err(format!(
                    "the new device has {} channels instead of {}",
                    format.channels, self.channels
                ))
            }
        });
        match result {
            Ok(format) => log::info!(
                "the output device changed; system audio moved to \"{}\" at {} Hz",
                format.device_name,
                format.sample_rate
            ),
            Err(error) => {
                log::error!("couldn't move system audio to the new output device: {error}");
                // The UI capitalizes the message and adds the final period.
                (self.on_error)(format!(
                    "system audio couldn't move to the new output device ({error}), so Meeting Participants stopped transcribing. Stop and start the recording to try again"
                ));
            }
        }
    }

    /// Starts a tap that sends to `frames`. After a rebuild, `gap_from` is when
    /// the old tap's last buffer arrived: the new tap first sends silence for
    /// the time since, up to `MAX_GAP`, so transcript times stay in step.
    fn start_tap(&mut self, gap_from: Option<Instant>) -> Result<Format, String> {
        let frames = self.frames.clone();
        let counter = self.dropped.clone();
        let last_buffer = self.last_buffer.clone();
        let (epoch, channels) = (self.epoch, self.channels);
        let mut gap_from = gap_from;
        let on_samples: OnSamples = Box::new(move |samples, sample_rate| {
            let send = |buffer: Buffer| {
                if let Err(TrySendError::Full(_)) = frames.try_send(buffer) {
                    counter.fetch_add(1, Ordering::Relaxed);
                }
            };
            if let Some(from) = gap_from.take() {
                let gap = silence(from.elapsed().min(MAX_GAP), sample_rate, channels);
                if !gap.is_empty() {
                    send(Buffer {
                        samples: gap,
                        sample_rate,
                    });
                }
            }
            send(Buffer {
                samples: samples.to_vec(),
                sample_rate,
            });
            last_buffer.store(epoch.elapsed().as_nanos() as u64, Ordering::Relaxed);
        });
        let (tap, format) = self.taps.start(on_samples)?;
        self.tap = Some(tap);
        if self.channels == 0 {
            self.channels = format.channels;
        }
        Ok(format)
    }
}

/// Interleaved silence lasting `duration`.
fn silence(duration: Duration, sample_rate: u32, channels: u16) -> Vec<f32> {
    let frames = (duration.as_secs_f64() * sample_rate as f64).round() as usize;
    vec![0.0; frames * channels as usize]
}

/// Decides whether to start capture from the permission status, asking first
/// when it's undecided. An unknown status lets capture proceed.
fn check_access(
    status: Option<Permission>,
    request: impl FnOnce() -> Option<bool>,
) -> Result<(), CaptureError> {
    match status {
        Some(Permission::Authorized) | None => Ok(()),
        Some(Permission::NotDetermined) => match request() {
            Some(false) => Err(CaptureError::SystemAudioPermissionDenied),
            Some(true) | None => Ok(()),
        },
        Some(Permission::Denied | Permission::Restricted) => {
            Err(CaptureError::SystemAudioPermissionDenied)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    type Slot = Arc<Mutex<Option<OnSamples>>>;
    /// What a fake start returns: the device's rate and channel count, or an error.
    type Start = Result<(u32, u16), String>;

    /// Fake Core Audio: each start makes a tap whose callback the test drives.
    #[derive(Clone, Default)]
    struct FakeTaps {
        /// One slot per tap started, holding its callback while the tap runs.
        taps: Arc<Mutex<Vec<Slot>>>,
        running: Arc<AtomicUsize>,
        /// Results for upcoming starts, in order: a rate and channel count, or an error.
        script: Arc<Mutex<Vec<Start>>>,
    }

    struct FakeTap {
        slot: Slot,
        running: Arc<AtomicUsize>,
    }

    impl Drop for FakeTap {
        fn drop(&mut self) {
            self.slot.lock().unwrap().take();
            self.running.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl TapStarter for FakeTaps {
        fn start(&mut self, on_samples: OnSamples) -> Result<(Box<dyn Send>, Format), String> {
            let (rate, channels) = self.script.lock().unwrap().remove(0)?;
            let slot = Arc::new(Mutex::new(Some(on_samples)));
            self.taps.lock().unwrap().push(slot.clone());
            self.running.fetch_add(1, Ordering::SeqCst);
            let format = Format {
                sample_rate: rate,
                tap_sample_rate: rate,
                channels,
                device_name: format!("Device {}", self.taps.lock().unwrap().len()),
            };
            let tap = FakeTap {
                slot,
                running: self.running.clone(),
            };
            Ok((Box::new(tap), format))
        }
    }

    impl FakeTaps {
        fn scripted(script: Vec<Start>) -> Self {
            let taps = FakeTaps::default();
            *taps.script.lock().unwrap() = script;
            taps
        }

        fn started(&self) -> usize {
            self.taps.lock().unwrap().len()
        }

        /// Delivers samples from the `index`th tap, as Core Audio would.
        fn deliver(&self, index: usize, samples: &[f32], rate: u32) {
            let slot = self.taps.lock().unwrap()[index].clone();
            let mut callback = slot.lock().unwrap();
            (callback.as_mut().expect("that tap was stopped"))(samples, rate);
        }

        fn wait_for_starts(&self, count: usize) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.started() < count {
                assert!(
                    Instant::now() < deadline,
                    "only {} taps started",
                    self.started()
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    /// A fake device watcher: `change` reports a device change, and `watching`
    /// is 1 until the capture drops it.
    #[derive(Clone, Default)]
    struct FakeWatch {
        on_change: Arc<Mutex<Option<OnChange>>>,
        watching: Arc<AtomicUsize>,
    }

    /// Like the real watcher, frees the change callback when dropped.
    struct WatchHandle(FakeWatch);

    impl Drop for WatchHandle {
        fn drop(&mut self) {
            self.0.on_change.lock().unwrap().take();
            self.0.watching.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl FakeWatch {
        fn starter(&self) -> WatchStarter {
            let this = self.clone();
            Box::new(move |on_change| {
                *this.on_change.lock().unwrap() = Some(on_change);
                this.watching.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(WatchHandle(this.clone())) as Box<dyn Send>)
            })
        }

        fn change(&self) {
            (self.on_change.lock().unwrap().as_ref().unwrap())();
        }
    }

    fn collect_issues() -> (ErrorSink, Arc<Mutex<Vec<String>>>) {
        let issues = Arc::new(Mutex::new(Vec::new()));
        let sink = issues.clone();
        (
            Arc::new(move |message| sink.lock().unwrap().push(message)),
            issues,
        )
    }

    fn follow(
        taps: &FakeTaps,
        watch: &FakeWatch,
    ) -> (SystemCapture, Receiver<Buffer>, Arc<Mutex<Vec<String>>>) {
        let (frames, received) = sync_channel(64);
        let (sink, issues) = collect_issues();
        let (capture, format) =
            start_following(taps.clone(), watch.starter(), frames, sink).unwrap();
        assert_eq!(
            format,
            CaptureFormat {
                sample_rate: 48_000,
                channels: 2
            }
        );
        (capture, received, issues)
    }

    #[test]
    fn moves_to_the_new_output_device_and_fills_the_gap_with_silence() {
        let taps = FakeTaps::scripted(vec![Ok((48_000, 2)), Ok((24_000, 2))]);
        let watch = FakeWatch::default();
        let (capture, received, issues) = follow(&taps, &watch);
        taps.deliver(0, &[0.5; 960], 48_000);

        // AirPods connect: the system reports the change, often more than once.
        watch.change();
        watch.change();
        taps.wait_for_starts(2);
        taps.deliver(1, &[0.25; 480], 24_000);

        assert_eq!(
            taps.running.load(Ordering::SeqCst),
            1,
            "the old tap is stopped"
        );
        let old = received.recv().unwrap();
        assert_eq!((old.samples.len(), old.sample_rate), (960, 48_000));
        let gap = received.recv().unwrap();
        assert_eq!(gap.sample_rate, 24_000);
        assert!(gap.samples.iter().all(|&s| s == 0.0));
        // At least the settling time passed between the two taps' buffers.
        let gap_ms = gap.samples.len() as u64 / 2 * 1000 / 24_000;
        assert!(
            (SETTLE.as_millis() as u64..2_000).contains(&gap_ms),
            "{gap_ms} ms"
        );
        let new = received.recv().unwrap();
        assert_eq!((new.samples[0], new.sample_rate), (0.25, 24_000));
        assert_eq!(taps.started(), 2, "two notifications rebuild once");
        assert!(issues.lock().unwrap().is_empty());
        drop(capture);
    }

    #[test]
    fn reports_an_issue_when_capture_cant_move_and_recovers_on_the_next_change() {
        let taps = FakeTaps::scripted(vec![
            Ok((48_000, 2)),
            Err("Couldn't create the capture device (OSStatus 1)".into()),
            Ok((48_000, 2)),
        ]);
        let watch = FakeWatch::default();
        let (_capture, received, issues) = follow(&taps, &watch);

        watch.change();
        let deadline = Instant::now() + Duration::from_secs(5);
        while issues.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "no issue reported");
            thread::sleep(Duration::from_millis(10));
        }
        let issue = issues.lock().unwrap()[0].clone();
        assert!(
            issue.contains("so Meeting Participants stopped transcribing"),
            "{issue}"
        );
        assert!(!issue.ends_with('.'), "the UI adds the period: {issue}");
        assert!(issue.contains("OSStatus 1"), "{issue}");
        assert_eq!(taps.running.load(Ordering::SeqCst), 0);

        watch.change();
        taps.wait_for_starts(2);
        taps.deliver(1, &[0.1; 96], 48_000);
        assert_eq!(received.recv().unwrap().sample_rate, 48_000);
    }

    #[test]
    fn refuses_a_new_device_with_a_different_channel_layout() {
        let taps = FakeTaps::scripted(vec![Ok((48_000, 2)), Ok((48_000, 1))]);
        let watch = FakeWatch::default();
        let (_capture, _received, issues) = follow(&taps, &watch);

        watch.change();
        taps.wait_for_starts(2);
        let deadline = Instant::now() + Duration::from_secs(5);
        while issues.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "no issue reported");
            thread::sleep(Duration::from_millis(10));
        }

        assert!(issues.lock().unwrap()[0].contains("1 channels instead of 2"));
        assert_eq!(taps.running.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stopping_stops_watching_and_the_tap_and_closes_the_stream() {
        let taps = FakeTaps::scripted(vec![Ok((48_000, 2))]);
        let watch = FakeWatch::default();
        let (capture, received, _issues) = follow(&taps, &watch);
        assert_eq!(watch.watching.load(Ordering::SeqCst), 1);

        drop(capture);

        assert_eq!(watch.watching.load(Ordering::SeqCst), 0);
        assert_eq!(taps.running.load(Ordering::SeqCst), 0);
        assert!(received.recv().is_err());
    }

    #[test]
    fn keeps_capturing_when_the_device_watcher_cant_start() {
        let taps = FakeTaps::scripted(vec![Ok((48_000, 2))]);
        let (frames, received) = sync_channel(4);
        let (sink, _issues) = collect_issues();
        let watch: WatchStarter = Box::new(|_| Err("no watching".into()));

        let (capture, _) = start_following(taps.clone(), watch, frames, sink).unwrap();
        taps.deliver(0, &[0.3; 96], 48_000);

        assert_eq!(received.recv().unwrap().samples.len(), 96);
        drop(capture);
    }

    fn never_asked() -> Option<bool> {
        panic!("shouldn't ask");
    }

    #[test]
    fn starts_when_allowed_or_unknown() {
        assert!(check_access(Some(Permission::Authorized), never_asked).is_ok());
        assert!(check_access(None, never_asked).is_ok());
    }

    #[test]
    fn asks_when_undecided_and_follows_the_answer() {
        assert!(check_access(Some(Permission::NotDetermined), || Some(true)).is_ok());
        assert!(matches!(
            check_access(Some(Permission::NotDetermined), || Some(false)),
            Err(CaptureError::SystemAudioPermissionDenied)
        ));
        // If macOS won't let the app ask, starting the tap shows the prompt instead.
        assert!(check_access(Some(Permission::NotDetermined), || None).is_ok());
    }

    #[test]
    fn refuses_when_denied_or_restricted() {
        for status in [Permission::Denied, Permission::Restricted] {
            assert!(matches!(
                check_access(Some(status), never_asked),
                Err(CaptureError::SystemAudioPermissionDenied)
            ));
        }
    }
}
