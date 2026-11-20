use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    io::{self, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
struct Chunk {
    index: u64,
    bytes: Vec<u8>,
}
#[derive(Clone)]
struct Forward {
    sender: SyncSender<Chunk>,
    samples: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
}
pub struct Shared {
    pub rate: u32,
    forward: Mutex<Option<Forward>>,
    pub level: AtomicU32,
    pub error: Mutex<Option<String>>,
}
pub struct Microphone {
    _stream: cpal::Stream,
    pub shared: Arc<Shared>,
}
impl Microphone {
    pub fn start_selected(selected: Option<&str>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = if let Some(name)=selected {
            let mut matches=host.input_devices().map_err(|e|e.to_string())?.filter(|d|d.name().ok().as_deref()==Some(name));
            let device=matches.next().ok_or("The selected microphone is unavailable. Reconnect it or select another.")?;
            if matches.next().is_some(){return Err("Several microphones have this name; choose the system default input.".into());}
            device
        } else {host.default_input_device().ok_or("No microphone is connected")?};
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let rate = config.sample_rate().0;
        if !(8000..=192000).contains(&rate) || config.channels() == 0 {
            return Err("Unsupported microphone configuration".into());
        }
        let state = Arc::new(Shared {
            rate,
            forward: Mutex::new(None),
            level: AtomicU32::new(0),
            error: Mutex::new(None),
        });
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config.into(), state.clone()),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config.into(), state.clone()),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config.into(), state.clone()),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config.into(), state.clone()),
            _ => Err("This microphone's sample format is unsupported".into()),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: stream,
            shared: state,
        })
    }
}
fn build<T: cpal::SizedSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    f32: cpal::FromSample<T>,
{
    let channels = config.channels as usize;
    let errors = shared.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let forward = shared.forward.try_lock().ok().and_then(|v| v.clone());
                let mut energy = 0.;
                let mut count = 0;
                for block in data.chunks(channels * 512) {
                    let mut bytes = Vec::with_capacity(if forward.is_some() {
                        block.len() / channels * 4
                    } else {
                        0
                    });
                    for frame in block.chunks_exact(channels) {
                        let value = frame
                            .iter()
                            .map(|s| <f32 as cpal::Sample>::from_sample(*s))
                            .sum::<f32>()
                            / channels as f32;
                        let value = if value.is_finite() {
                            value.clamp(-1., 1.)
                        } else {
                            0.
                        };
                        energy += value * value;
                        count += 1;
                        if forward.is_some() {
                            bytes.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                    if let Some(target) = &forward {
                        let samples = (bytes.len() / 4) as u64;
                        let index = target.samples.fetch_add(samples, Ordering::Relaxed);
                        if target.sender.try_send(Chunk { index, bytes }).is_err() {
                            target.dropped.fetch_add(samples, Ordering::Relaxed);
                        }
                    }
                }
                let rms = (energy / (count.max(1) as f32)).sqrt();
                shared.level.store(rms.to_bits(), Ordering::Relaxed);
            },
            move |error| {
                if let Ok(mut value) = errors.error.lock() {
                    *value = Some(error.to_string());
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}
pub struct Session {
    pub address: String,
    pub rate: u32,
    source: Arc<Shared>,
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    writer: Option<thread::JoinHandle<()>>,
    pub dropped: Arc<AtomicU64>,
}
impl Session {
    /// `origin` is the instant the first video frame was handed over. The encoder connects here only after it has
    /// started reading video, so the first live sample is placed at its distance from that instant (the writer fills
    /// the gap with silence); placed at zero, the sound would lead the picture by the encoder's start-up time.
    pub fn new(source: Arc<Shared>, origin: Arc<OnceLock<Instant>>) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?.to_string();
        let (tx, rx) = mpsc::sync_channel::<Chunk>(4);
        let dropped = Arc::new(AtomicU64::new(0));
        let lease = Forward {
            sender: tx,
            samples: Arc::new(AtomicU64::new(0)),
            dropped: dropped.clone(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let error = Arc::new(Mutex::new(None));
        let outcome = error.clone();
        let input = source.clone();
        let writer = thread::spawn(move || {
            let result = (|| -> io::Result<()> {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut socket: TcpStream = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            if cancelled.load(Ordering::Relaxed) {
                                return Ok(());
                            }
                            if Instant::now() > deadline {
                                return Err(io::Error::other("Microphone encoder did not connect"));
                            }
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => return Err(e),
                    }
                };
                // On macOS (and the BSDs) the accepted socket inherits the listener's non-blocking mode; a full send
                // buffer then failed the take at once with "Resource temporarily unavailable" when its send buffer filled.
                socket.set_nonblocking(false)?;
                if let Some(origin) = origin.get() {
                    lease.samples.store((origin.elapsed().as_secs_f64() * f64::from(input.rate)) as u64, Ordering::Relaxed);
                }
                *input
                    .forward
                    .lock()
                    .map_err(|_| io::Error::other("Microphone lock poisoned"))? = Some(lease);
                // A short timeout only bounds each wait, so a stop or a stuck encoder is noticed (see `send`).
                socket.set_write_timeout(Some(WRITE_POLL))?;
                socket.set_nodelay(true)?;
                let mut expected = 0;
                let mut finishing = None;
                let silence = [0u8; 4096];
                // Until stopped and nothing is left queued: samples captured before Stop still reach the file.
                loop {
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(chunk) => {
                            let mut gap = chunk.index.saturating_sub(expected) * 4;
                            while gap > 0 {
                                let amount = (gap as usize).min(silence.len());
                                send(&mut socket, &silence[..amount], &cancelled, &mut finishing)?;
                                gap -= amount as u64;
                            }
                            send(&mut socket, &chunk.bytes, &cancelled, &mut finishing)?;
                            expected = chunk.index + (chunk.bytes.len() / 4) as u64;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) if cancelled.load(Ordering::Relaxed) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                Ok(())
            })();
            if let Err(e) = result {
                if let Ok(mut value) = outcome.lock() {
                    *value = Some(e.to_string());
                }
            }
        });
        let rate = source.rate;
        Ok(Self {
            address,
            rate,
            source,
            stop,
            error,
            writer: Some(writer),
            dropped,
        })
    }
    pub fn check(&self) -> io::Result<()> {
        for error in [&self.error, &self.source.error] {
            if let Ok(value) = error.lock() {
                if let Some(message) = &*value {
                    return Err(io::Error::other(message.clone()));
                }
            }
        }
        Ok(())
    }
    pub fn finish(&mut self) -> io::Result<()> {
        if let Ok(mut value) = self.source.forward.lock() {
            *value = None;
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(writer) = self.writer.take() {
            writer
                .join()
                .map_err(|_| io::Error::other("Microphone writer failed"))?;
        }
        self.check()
    }
}
/// How long one socket write may wait before the writer looks again at whether it is stopping or stuck.
const WRITE_POLL: Duration = Duration::from_millis(250);
/// An encoder that takes no audio at all for this long has stopped (the same limit as the video watchdog). While
/// finishing, the limit is shorter, so the microphone always closes inside the pipeline's own 3 s finish bound.
const STUCK_RECORDING: Duration = Duration::from_secs(5);
const STUCK_FINISHING: Duration = Duration::from_secs(2);
/// Writes all of `bytes`, continuing from wherever a partial write stopped (nothing is sent twice). An encoder that is
/// busy for a while (its socket buffers full) is waited for; one that accepts nothing for the stuck limit is an error,
/// and so is any real socket failure (closed, reset).
fn send(socket: &mut TcpStream, bytes: &[u8], stopping: &AtomicBool, finishing: &mut Option<Instant>) -> io::Result<()> {
    let (mut sent, mut progress) = (0, Instant::now());
    while sent < bytes.len() {
        // One finish deadline across all queued chunks and silence, even if a slow reader makes partial progress.
        if stopping.load(Ordering::Relaxed) && finishing.get_or_insert_with(Instant::now).elapsed() >= STUCK_FINISHING {
            return Err(io::Error::other("The microphone audio did not finish draining within 2 seconds"));
        }
        match socket.write(&bytes[sent..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                sent += n;
                progress = Instant::now();
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {
                if progress.elapsed() >= STUCK_RECORDING {
                    return Err(io::Error::other("The encoder stopped taking microphone audio for 5 seconds"));
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// Deterministic tone for qualification; never opens a physical microphone.
pub struct TestTone {
    pub shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl TestTone {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            rate: 48000,
            forward: Mutex::new(None),
            level: AtomicU32::new(0),
            error: Mutex::new(None),
        });
        let source = shared.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::spawn(move || {
            let mut index = 0u64;
            let epoch = Instant::now();
            while !flag.load(Ordering::Relaxed) {
                if let Some(target) = source.forward.lock().ok().and_then(|v| v.clone()) {
                    let mut bytes = Vec::with_capacity(1920);
                    for i in 0..480 {
                        let sample = (((index + i) as f32 * 440. * std::f32::consts::TAU / 48000.)
                            .sin()
                            * 0.1)
                            .to_le_bytes();
                        bytes.extend_from_slice(&sample);
                    }
                    let n = target.samples.fetch_add(480, Ordering::Relaxed);
                    if target.sender.try_send(Chunk { index: n, bytes }).is_err() {
                        target.dropped.fetch_add(480, Ordering::Relaxed);
                    }
                }
                index += 480;
                thread::sleep(
                    (epoch + Duration::from_secs_f64(index as f64 / 48000.))
                        .saturating_duration_since(Instant::now()),
                );
            }
        });
        Self {
            shared,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for TestTone {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod backpressure_tests {
    use super::*;
    use std::io::Read;
    /// Sample `index` of the test stream. Never zero, so silence is told apart from sound, and a sample that is
    /// repeated or out of place is caught at its position.
    fn sample(index: u64) -> f32 {
        (index % 997 + 1) as f32 / 1000.
    }
    /// An encoder that stops reading the microphone until the loopback buffers are full and the queue has overflowed,
    /// and then reads again. The take must go on, and the stream must stay exactly as long as the samples it was given:
    /// whatever could not be queued meanwhile becomes silence of the same length, and nothing partly written is
    /// written again. How much the buffers hold differs by system and settings (a Linux VM with default autotuning held
    /// about 15 s of this audio), so the source fills them at once and the pause lasts until the writer is seen stuck,
    /// rather than for a fixed time. Before the fix the accepted socket was non-blocking and the first full buffer failed
    /// the take.
    #[test]
    fn a_paused_encoder_is_waited_for_and_the_stream_keeps_its_length() {
        let source = Arc::new(Shared {
            rate: 48000,
            forward: Mutex::new(None),
            level: AtomicU32::new(0),
            error: Mutex::new(None),
        });
        let origin = Arc::new(OnceLock::new());
        origin.set(Instant::now()).unwrap();
        let mut session = Session::new(source.clone(), origin).unwrap();
        let mut encoder = TcpStream::connect(&session.address).unwrap();
        let (resume, paused) = mpsc::channel::<()>();
        let reader = thread::spawn(move || {
            let mut all = Vec::new();
            // A failed test drops `resume`; the reader then just ends, adding no second panic.
            if paused.recv().is_ok() {
                encoder.read_to_end(&mut all).unwrap();
            }
            all
        });
        let connecting = Instant::now();
        let target = loop {
            if let Some(target) = source.forward.lock().unwrap().clone() {
                break target;
            }
            assert!(connecting.elapsed() < Duration::from_secs(10), "the writer never took the connection: {:?}", session.check());
            thread::sleep(Duration::from_millis(1));
        };
        // Silence the writer places before the first sample (the encoder's start-up after `origin`).
        let start = target.samples.load(Ordering::Relaxed);
        let next = || {
            let index = target.samples.fetch_add(480, Ordering::Relaxed);
            Chunk { index, bytes: (index..index + 480).flat_map(|i| sample(i).to_le_bytes()).collect() }
        };
        // Hands over chunks without losing any while the writer keeps up. Returns the chunk still waiting once the queue
        // has stayed full for `stuck`: the writer is then waiting on full buffers.
        let deliver = |count: usize, stuck: Option<Duration>| {
            let (mut accepted, mut full_since, mut waiting) = (0, None::<Instant>, None);
            let deadline = Instant::now() + Duration::from_secs(30);
            while accepted < count {
                assert!(Instant::now() < deadline, "the writer took no stream for 30 s: {:?}", session.check());
                match target.sender.try_send(waiting.take().unwrap_or_else(next)) {
                    Ok(()) => (accepted, full_since) = (accepted + 1, None),
                    Err(mpsc::TrySendError::Full(chunk)) => {
                        if stuck.is_some_and(|limit| full_since.get_or_insert_with(Instant::now).elapsed() >= limit) {
                            return Some(chunk);
                        }
                        waiting = Some(chunk);
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(e) => panic!("the writer went away ({e}): {:?}", session.check()),
                }
            }
            None
        };
        // Busy for half the time after which an encoder counts as stuck: many socket timeouts, but not a failure.
        let blocked = deliver(usize::MAX, Some(STUCK_RECORDING / 2)).unwrap();
        // The overflow: like the microphone callback, a chunk that cannot be queued is counted as dropped.
        for chunk in std::iter::once(blocked).chain(std::iter::repeat_with(next).take(9)) {
            let samples = (chunk.bytes.len() / 4) as u64;
            assert!(target.sender.try_send(chunk).is_err(), "the pause really did overflow the queue");
            target.dropped.fetch_add(samples, Ordering::Relaxed);
        }
        assert!(session.check().is_ok(), "a busy encoder is not a failure: {:?}", session.check());
        resume.send(()).unwrap();
        deliver(10, None);
        let total = target.samples.load(Ordering::Relaxed);
        // A sender is still held, as a microphone callback may: the writer must stop on `finish`, not on a closed queue.
        // It is let go after 3 s, so a writer that ignores the stop fails the bound below instead of hanging the test.
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(3));
            drop(target);
        });
        let finishing = Instant::now();
        session.finish().unwrap();
        assert!(finishing.elapsed() < Duration::from_secs(1), "and it stops promptly once it reads again");
        let bytes = reader.join().unwrap();
        assert_eq!(bytes.len() as u64, total * 4, "the stream is exactly as long as the samples it was given");
        let mut silent = 0;
        for (index, value) in bytes.chunks_exact(4).enumerate() {
            match f32::from_le_bytes(value.try_into().unwrap()) {
                0. => silent += 1,
                value => assert_eq!(value, sample(index as u64), "sample {index} is out of place"),
            }
        }
        let dropped = session.dropped.load(Ordering::Relaxed);
        assert_eq!(silent, start + dropped, "only the start-up and the overflow became silence");
    }
}
