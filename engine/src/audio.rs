use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    io::{self, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
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
    pub fn new(source: Arc<Shared>) -> io::Result<Self> {
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
                *input
                    .forward
                    .lock()
                    .map_err(|_| io::Error::other("Microphone lock poisoned"))? = Some(lease);
                socket.set_write_timeout(Some(Duration::from_secs(2)))?;
                socket.set_nodelay(true)?;
                let mut expected = 0;
                let silence = [0u8; 4096];
                while !cancelled.load(Ordering::Relaxed) {
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(chunk) => {
                            let mut gap = chunk.index.saturating_sub(expected) * 4;
                            while gap > 0 {
                                let amount = (gap as usize).min(silence.len());
                                socket.write_all(&silence[..amount])?;
                                gap -= amount as u64;
                            }
                            socket.write_all(&chunk.bytes)?;
                            expected = chunk.index + (chunk.bytes.len() / 4) as u64;
                        }
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
