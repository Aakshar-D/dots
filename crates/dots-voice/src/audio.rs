//! The real microphone and speaker, through the operating system's default audio host.
//!
//! No sample is written to disk or to logs. Each cpal stream lives on a thread the struct
//! owns, because a stream is not `Send` on every platform.

use crate::ears::level;
use crate::engine::{resample, Clip, EngineError, EAR_RATE, FRAME};
use crate::voice::Sink;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, Sample, SampleFormat, SizedSample};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{
    channel, sync_channel, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError,
};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

/// Frames the microphone keeps for a slow consumer: 64 x 32 ms, about 2 seconds.
const MIC_QUEUE: usize = 64;
/// How long the speaker fades out at `stop`.
const FADE_MS: usize = 5;
/// How much of the latest playback `level` looks at.
const LEVEL_MS: usize = 30;
/// How often a stream-owning thread looks for a stop request or a lost stream.
const WATCH: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------------------
// Devices

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Stable on this machine and host; pass it back to `open`.
    pub id: String,
    pub name: String,
    pub default: bool,
}

fn list(input: bool) -> Vec<Device> {
    let host = cpal::default_host();
    let default_id = if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    }
    .and_then(|d| d.id().ok())
    .map(|id| id.to_string());
    let devices: Vec<cpal::Device> = if input {
        host.input_devices().map(|d| d.collect())
    } else {
        host.output_devices().map(|d| d.collect())
    }
    .unwrap_or_default();
    devices
        .iter()
        .filter_map(|d| {
            let id = d.id().ok()?.to_string();
            let name = d
                .description()
                .map(|desc| desc.name().to_string())
                .unwrap_or_else(|_| id.clone());
            Some(Device {
                default: default_id.as_deref() == Some(id.as_str()),
                id,
                name,
            })
        })
        .collect()
}

pub fn microphones() -> Vec<Device> {
    list(true)
}

pub fn speakers() -> Vec<Device> {
    list(false)
}

/// The device with this id, or the default one when `id` is `None`, unknown or gone.
fn pick(input: bool, id: Option<&str>) -> Result<cpal::Device, EngineError> {
    let host = cpal::default_host();
    let named = id
        .and_then(|s| s.parse::<cpal::DeviceId>().ok())
        .and_then(|id| host.device_by_id(&id))
        .filter(|d| {
            if input {
                d.supports_input()
            } else {
                d.supports_output()
            }
        });
    let found = named.or_else(|| {
        if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        }
    });
    found.ok_or_else(|| {
        EngineError::Failed(
            if input {
                "no microphone was found"
            } else {
                "no speaker was found"
            }
            .into(),
        )
    })
}

fn describe(e: &cpal::Error) -> EngineError {
    match e.kind() {
        ErrorKind::PermissionDenied => {
            EngineError::Failed("access to the audio device was denied".into())
        }
        ErrorKind::DeviceBusy => EngineError::Failed("the audio device is busy".into()),
        ErrorKind::DeviceNotAvailable => {
            EngineError::Failed("the audio device is not available".into())
        }
        _ => EngineError::Failed(e.to_string()),
    }
}

/// Errors that leave the stream running.
fn harmless(e: &cpal::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::DeviceChanged | ErrorKind::Xrun | ErrorKind::RealtimeDenied
    )
}

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Builds the stream for whichever sample type the device gives.
macro_rules! by_format {
    ($format:expr, $build:ident($($arg:expr),*)) => {
        match $format {
            SampleFormat::F32 => $build::<f32>($($arg),*),
            SampleFormat::I16 => $build::<i16>($($arg),*),
            SampleFormat::U16 => $build::<u16>($($arg),*),
            SampleFormat::I32 => $build::<i32>($($arg),*),
            SampleFormat::U8 => $build::<u8>($($arg),*),
            SampleFormat::F64 => $build::<f64>($($arg),*),
            _ => Err(cpal::Error::from(ErrorKind::UnsupportedConfig)),
        }
    };
}

// ---------------------------------------------------------------------------------------
// Microphone

pub enum MicFrame {
    /// `FRAME` samples of 16 kHz mono audio.
    Audio(Vec<f32>),
    /// The microphone stopped working; nothing follows.
    Lost(String),
}

pub trait Mic: Send {
    /// The stream of frames. Taken once; later calls return a receiver that is already closed.
    fn frames(&mut self) -> Receiver<MicFrame>;
}

/// Mono, averaged over the channels of each interleaved frame.
fn downmix(samples: &[f32], channels: u16) -> Vec<f32> {
    let ch = usize::from(channels.max(1));
    if ch == 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(ch)
        .map(|f| f.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// Turns what a device delivers into `FRAME`-sized pieces of 16 kHz mono audio. What does not
/// fill a frame stays in `carry` (already 16 kHz mono) for the next call.
pub fn frames_from(
    samples: &[f32],
    rate: u32,
    channels: u16,
    carry: &mut Vec<f32>,
) -> Vec<Vec<f32>> {
    let mono = downmix(samples, channels);
    if rate == EAR_RATE {
        carry.extend_from_slice(&mono);
    } else {
        carry.extend(
            resample(
                &Clip {
                    samples: mono,
                    rate,
                },
                EAR_RATE,
            )
            .samples,
        );
    }
    let whole = carry.len() / FRAME * FRAME;
    let frames = carry[..whole].chunks(FRAME).map(<[f32]>::to_vec).collect();
    carry.drain(..whole);
    frames
}

/// The microphone. Dropping it stops the stream and gives the device back.
pub struct CpalMic {
    frames: Option<Receiver<MicFrame>>,
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl CpalMic {
    /// `None`, or an id that is no longer there, opens the default microphone.
    pub fn open(device_id: Option<&str>) -> Result<Self, EngineError> {
        let (tx, rx) = sync_channel(MIC_QUEUE);
        let (init_tx, init_rx) = channel();
        let (stop_tx, stop_rx) = channel();
        let id = device_id.map(str::to_owned);
        let thread = std::thread::Builder::new()
            .name("dots-voice-mic".into())
            .spawn(move || mic_thread(id, tx, init_tx, stop_rx))
            .map_err(|e| EngineError::Failed(e.to_string()))?;
        match init_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                frames: Some(rx),
                stop: Some(stop_tx),
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(EngineError::Failed("the microphone thread ended".into())),
        }
    }
}

impl Mic for CpalMic {
    fn frames(&mut self) -> Receiver<MicFrame> {
        self.frames.take().unwrap_or_else(|| sync_channel(1).1)
    }
}

impl Drop for CpalMic {
    fn drop(&mut self) {
        drop(self.stop.take()); // the thread sees the closed channel and lets go of the device
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

type Lost = Arc<Mutex<Option<String>>>;

fn mic_thread(
    id: Option<String>,
    tx: SyncSender<MicFrame>,
    init: Sender<Result<(), EngineError>>,
    stop: Receiver<()>,
) {
    let lost: Lost = Arc::new(Mutex::new(None));
    let stream = match open_input(id.as_deref(), tx.clone(), lost.clone()) {
        Ok(stream) => {
            let _ = init.send(Ok(()));
            stream
        }
        Err(e) => {
            let _ = init.send(Err(e));
            return;
        }
    };
    loop {
        if !matches!(stop.recv_timeout(WATCH), Err(RecvTimeoutError::Timeout)) {
            return; // asked to stop; the stream is dropped on the way out
        }
        let message = locked(&lost).take();
        if let Some(message) = message {
            drop(stream);
            let mut pending = Some(MicFrame::Lost(message));
            while let Some(frame) = pending.take() {
                if let Err(TrySendError::Full(frame)) = tx.try_send(frame) {
                    // the consumer is behind; wait for room unless told to stop
                    if matches!(
                        stop.recv_timeout(Duration::from_millis(10)),
                        Err(RecvTimeoutError::Timeout)
                    ) {
                        pending = Some(frame);
                    }
                }
            }
            return;
        }
    }
}

fn open_input(
    id: Option<&str>,
    tx: SyncSender<MicFrame>,
    lost: Lost,
) -> Result<cpal::Stream, EngineError> {
    let device = pick(true, id)?;
    let supported = device.default_input_config().map_err(|e| describe(&e))?;
    let format = supported.sample_format();
    let (rate, channels) = (supported.sample_rate(), supported.channels());
    let config = supported.config();
    let stream = by_format!(
        format,
        build_input(&device, config, rate, channels, tx, lost)
    )
    .map_err(|e| describe(&e))?;
    stream.play().map_err(|e| describe(&e))?;
    Ok(stream)
}

fn build_input<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    rate: u32,
    channels: u16,
    tx: SyncSender<MicFrame>,
    lost: Lost,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let mut carry = Vec::new();
    let mut floats: Vec<f32> = Vec::new();
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _| {
            floats.clear();
            floats.extend(data.iter().map(|s| f32::from_sample(*s)));
            for frame in frames_from(&floats, rate, channels, &mut carry) {
                // never block the audio thread: a full queue drops the newest frame
                let _ = tx.try_send(MicFrame::Audio(frame));
            }
        },
        move |e: cpal::Error| {
            if !harmless(&e) {
                locked(&lost).get_or_insert_with(|| e.to_string());
            }
        },
        None,
    )
}

// ---------------------------------------------------------------------------------------
// Speaker

/// The gain ramp that takes the sound down over `FADE_MS` after a stop.
#[derive(Debug)]
struct FadeState {
    /// Samples (per channel) in a full fade.
    len: usize,
    gain: f32,
    falling: bool,
}

impl FadeState {
    fn new(rate: u32) -> Self {
        Self {
            len: (rate as usize * FADE_MS / 1000).max(1),
            gain: 1.0,
            falling: false,
        }
    }

    /// Keeps only as much of `queue` as the fade needs to reach silence from the gain now.
    fn begin(&mut self, queue: &mut VecDeque<f32>) {
        let steps = ((self.gain * self.len as f32).ceil() as usize).max(1);
        queue.truncate(steps);
        self.falling = !queue.is_empty();
        if !self.falling {
            self.gain = 1.0;
        }
    }

    fn next_gain(&mut self) -> f32 {
        if !self.falling {
            return 1.0;
        }
        let gain = self.gain;
        self.gain -= 1.0 / self.len as f32;
        if self.gain <= 0.5 / self.len as f32 {
            self.reset();
        }
        gain
    }

    fn reset(&mut self) {
        self.gain = 1.0;
        self.falling = false;
    }
}

/// Writes the next `out.len() / channels` frames: one queued sample to every channel, silence
/// once the queue is dry.
fn fill(out: &mut [f32], channels: u16, queue: &mut VecDeque<f32>, fade: &mut FadeState) {
    let mut frames = out.chunks_exact_mut(usize::from(channels.max(1)));
    for frame in &mut frames {
        let sample = match queue.pop_front() {
            Some(s) => s * fade.next_gain(),
            None => {
                fade.reset();
                0.0
            }
        };
        frame.fill(sample);
    }
    frames.into_remainder().fill(0.0);
}

/// The most recent samples played, for `level`.
struct Window {
    samples: VecDeque<f32>,
    cap: usize,
}

impl Window {
    fn push_all<'a>(&mut self, samples: impl Iterator<Item = &'a f32>) {
        for s in samples {
            if self.samples.len() == self.cap {
                self.samples.pop_front();
            }
            self.samples.push_back(*s);
        }
    }

    fn level(&mut self) -> f32 {
        level(self.samples.make_contiguous())
    }
}

struct Playback {
    queue: VecDeque<f32>,
    fade: FadeState,
}

struct SinkShared {
    playback: Mutex<Playback>,
    /// `f32` bits of the last level the output callback worked out.
    level: AtomicU32,
    /// The output stream could not be opened or broke; clips are dropped.
    failed: AtomicBool,
}

/// The speaker. The output stream opens on the first `play` and stays open until the sink is
/// dropped.
pub struct CpalSink {
    shared: Arc<SinkShared>,
    rate: u32,
    start: Option<Sender<()>>,
    started: AtomicBool,
    thread: Option<JoinHandle<()>>,
}

impl CpalSink {
    /// `None`, or an id that is no longer there, uses the default speaker. Nothing sounds, and
    /// no stream opens, until the first `play`.
    pub fn open(device_id: Option<&str>) -> Result<Self, EngineError> {
        let (init_tx, init_rx) = channel();
        let (start_tx, start_rx) = channel();
        let id = device_id.map(str::to_owned);
        let thread = std::thread::Builder::new()
            .name("dots-voice-speaker".into())
            .spawn(move || sink_thread(id, init_tx, start_rx))
            .map_err(|e| EngineError::Failed(e.to_string()))?;
        match init_rx.recv() {
            Ok(Ok((shared, rate))) => Ok(Self {
                shared,
                rate,
                start: Some(start_tx),
                started: AtomicBool::new(false),
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(EngineError::Failed("the speaker thread ended".into())),
        }
    }
}

impl Sink for CpalSink {
    fn play(&self, clip: Clip) {
        if clip.samples.is_empty() || self.shared.failed.load(Ordering::Relaxed) {
            return;
        }
        let clip = resample(&clip, self.rate);
        if !self.started.swap(true, Ordering::SeqCst) {
            if let Some(start) = &self.start {
                let _ = start.send(());
            }
        }
        locked(&self.shared.playback).queue.extend(clip.samples);
    }

    fn stop(&self) {
        let mut p = locked(&self.shared.playback);
        let Playback { queue, fade } = &mut *p;
        fade.begin(queue);
    }

    fn idle(&self) -> bool {
        locked(&self.shared.playback).queue.is_empty()
    }

    fn level(&self) -> f32 {
        if self.idle() {
            0.0
        } else {
            f32::from_bits(self.shared.level.load(Ordering::Relaxed))
        }
    }
}

impl Drop for CpalSink {
    fn drop(&mut self) {
        drop(self.start.take()); // the thread sees the closed channel and drops the stream
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

type SinkInit = Result<(Arc<SinkShared>, u32), EngineError>;

fn sink_thread(id: Option<String>, init: Sender<SinkInit>, start: Receiver<()>) {
    let prepared = pick(false, id.as_deref()).and_then(|device| {
        let config = device.default_output_config().map_err(|e| describe(&e))?;
        Ok((device, config))
    });
    let (device, supported) = match prepared {
        Ok(p) => p,
        Err(e) => {
            let _ = init.send(Err(e));
            return;
        }
    };
    let rate = supported.sample_rate();
    let shared = Arc::new(SinkShared {
        playback: Mutex::new(Playback {
            queue: VecDeque::new(),
            fade: FadeState::new(rate),
        }),
        level: AtomicU32::new(0f32.to_bits()),
        failed: AtomicBool::new(false),
    });
    let _ = init.send(Ok((shared.clone(), rate)));
    if start.recv().is_err() {
        return; // dropped without ever playing
    }
    match open_output(&device, &supported, &shared) {
        Ok(stream) => {
            let _ = start.recv(); // waits for the sink to be dropped
            drop(stream);
        }
        Err(()) => fail(&shared),
    }
}

fn fail(shared: &SinkShared) {
    shared.failed.store(true, Ordering::Relaxed);
    locked(&shared.playback).queue.clear();
}

fn open_output(
    device: &cpal::Device,
    supported: &cpal::SupportedStreamConfig,
    shared: &Arc<SinkShared>,
) -> Result<cpal::Stream, ()> {
    let (rate, channels) = (supported.sample_rate(), supported.channels());
    let config = supported.config();
    let stream = by_format!(
        supported.sample_format(),
        build_output(device, config, rate, channels, shared.clone())
    )
    .map_err(|_| ())?;
    stream.play().map_err(|_| ())?;
    Ok(stream)
}

fn build_output<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    rate: u32,
    channels: u16,
    shared: Arc<SinkShared>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let mut scratch: Vec<f32> = Vec::new();
    let mut window = Window {
        samples: VecDeque::new(),
        cap: (rate as usize * LEVEL_MS / 1000).max(1),
    };
    let on_error = shared.clone();
    let ch = usize::from(channels.max(1));
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _| {
            scratch.resize(data.len(), 0.0); // grows once, to the device's buffer size
            {
                let mut p = locked(&shared.playback);
                let Playback { queue, fade } = &mut *p;
                fill(&mut scratch, channels, queue, fade);
            }
            window.push_all(scratch.iter().step_by(ch));
            shared
                .level
                .store(window.level().to_bits(), Ordering::Relaxed);
            for (out, s) in data.iter_mut().zip(&scratch) {
                *out = T::from_sample(*s);
            }
        },
        move |e: cpal::Error| {
            if !harmless(&e) {
                fail(&on_error);
            }
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fade(rate: u32) -> FadeState {
        FadeState::new(rate)
    }

    fn convert<T: Sample>(data: &[T]) -> Vec<f32>
    where
        f32: FromSample<T>,
    {
        data.iter().map(|s| f32::from_sample(*s)).collect()
    }

    #[test]
    fn stereo_48k_becomes_mono_16k_frames() {
        let samples: Vec<f32> = (0..96_000)
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect();
        let mut carry = Vec::new();
        let frames = frames_from(&samples, 48_000, 2, &mut carry);
        assert_eq!(frames.len(), 31);
        assert!(frames.iter().all(|f| f.len() == FRAME));
        assert!(frames.iter().flatten().all(|s| s.abs() < 1e-6));
        assert_eq!(carry.len(), 128);
    }

    #[test]
    fn the_remainder_is_carried_to_the_next_call() {
        let samples: Vec<f32> = (0..9_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut whole_carry = Vec::new();
        let whole = frames_from(&samples, 16_000, 1, &mut whole_carry);
        let mut carry = Vec::new();
        let mut parts = frames_from(&samples[..3_333], 16_000, 1, &mut carry);
        parts.extend(frames_from(&samples[3_333..], 16_000, 1, &mut carry));
        assert_eq!(parts, whole);
        assert_eq!(carry, whole_carry);
        // and with a resampled stereo input, split on a whole number of output samples
        let stereo: Vec<f32> = (0..57_600).map(|i| (i as f32 * 0.003).sin()).collect();
        let mut c1 = Vec::new();
        let one = frames_from(&stereo, 48_000, 2, &mut c1);
        let mut c2 = Vec::new();
        let mut two = frames_from(&stereo[..28_800], 48_000, 2, &mut c2);
        two.extend(frames_from(&stereo[28_800..], 48_000, 2, &mut c2));
        assert_eq!(one.len(), two.len());
        for (a, b) in one.iter().flatten().zip(two.iter().flatten()) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn already_16k_mono_is_only_cut() {
        let samples: Vec<f32> = (0..FRAME * 2 + 10).map(|i| i as f32 / 4096.0).collect();
        let mut carry = Vec::new();
        let frames = frames_from(&samples, 16_000, 1, &mut carry);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], samples[..FRAME]);
        assert_eq!(frames[1], samples[FRAME..FRAME * 2]);
        assert_eq!(carry, samples[FRAME * 2..]);
    }

    #[test]
    fn sample_formats_become_floats_in_range() {
        let i = convert(&[i16::MIN, 0, i16::MAX]);
        assert_eq!(i[0], -1.0);
        assert_eq!(i[1], 0.0);
        assert!((i[2] - 1.0).abs() < 1e-3);
        let u = convert(&[0u16, 32_768, u16::MAX]);
        assert_eq!(u[0], -1.0);
        assert_eq!(u[1], 0.0);
        assert!((u[2] - 1.0).abs() < 1e-3);
        assert_eq!(convert(&[0.25f32, -0.5]), vec![0.25, -0.5]);
    }

    #[test]
    fn channels_are_averaged() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        let three = downmix(&[0.3, 0.6, 0.9, 0.0, 0.0, 0.0], 3);
        assert!((three[0] - 0.6).abs() < 1e-6 && three[1] == 0.0);
        assert_eq!(downmix(&[0.1, 0.2], 1), vec![0.1, 0.2]);
    }

    #[test]
    fn fill_plays_each_sample_on_every_channel() {
        let mut queue: VecDeque<f32> = [0.1, 0.2, 0.3].into();
        let mut out = [9.0; 6];
        fill(&mut out, 2, &mut queue, &mut fade(48_000));
        assert_eq!(out, [0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
        assert!(queue.is_empty());
    }

    #[test]
    fn fill_pads_with_silence_when_the_queue_runs_dry() {
        let mut queue: VecDeque<f32> = [0.5].into();
        let mut out = [9.0; 5]; // two stereo frames and a stray sample
        fill(&mut out, 2, &mut queue, &mut fade(48_000));
        assert_eq!(out, [0.5, 0.5, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_stop_fades_to_zero_within_five_milliseconds_and_empties_the_queue() {
        let mut state = fade(48_000); // 240 samples
        let mut queue: VecDeque<f32> = std::iter::repeat_n(1.0, 10_000).collect();
        state.begin(&mut queue);
        assert_eq!(queue.len(), 240);
        let mut out = vec![9.0; 600];
        fill(&mut out, 1, &mut queue, &mut state);
        assert!(queue.is_empty());
        assert_eq!(out[0], 1.0);
        assert!(out[..240].windows(2).all(|w| w[1] < w[0]), "falls smoothly");
        assert!(out[239] < 0.01, "ends near zero: {}", out[239]);
        assert!(out[240..].iter().all(|s| *s == 0.0));
        // and sound that comes after plays at full volume again
        queue.extend([0.5, 0.5]);
        let mut next = [0.0; 2];
        fill(&mut next, 1, &mut queue, &mut state);
        assert_eq!(next, [0.5, 0.5]);
    }

    #[test]
    fn a_second_stop_during_the_fade_does_not_jump_back_up() {
        let mut state = fade(48_000);
        let mut queue: VecDeque<f32> = std::iter::repeat_n(1.0, 10_000).collect();
        state.begin(&mut queue);
        let mut first = vec![0.0; 100];
        fill(&mut first, 1, &mut queue, &mut state);
        state.begin(&mut queue);
        let mut rest = vec![0.0; 200];
        fill(&mut rest, 1, &mut queue, &mut state);
        let all: Vec<f32> = first.into_iter().chain(rest).collect();
        assert!(all.windows(2).all(|w| w[1] <= w[0]), "never rises");
        assert!(queue.is_empty());
    }

    #[test]
    fn a_stop_with_nothing_playing_changes_nothing() {
        let mut state = fade(48_000);
        let mut queue = VecDeque::new();
        state.begin(&mut queue);
        queue.push_back(0.7);
        let mut out = [0.0];
        fill(&mut out, 1, &mut queue, &mut state);
        assert_eq!(out, [0.7]);
    }

    #[test]
    fn the_level_window_follows_the_latest_samples() {
        let mut w = Window {
            samples: VecDeque::new(),
            cap: 100,
        };
        w.push_all(std::iter::repeat_n(&0.5f32, 100));
        assert!(w.level() > 0.8);
        w.push_all(std::iter::repeat_n(&0.0f32, 100));
        assert_eq!(w.level(), 0.0);
    }
}
