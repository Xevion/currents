use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::{fs, panic};

use image::RgbImage;
use snafu::ResultExt;

use super::encode::{Animation, Encoding, encode};
use super::{CreateDirSnafu, EncodeSnafu, Error, WriteSnafu};

/// Most threads that encode stills at once; the renderer cannot feed more than this.
const MAX_WORKERS: usize = 8;

/// A frame and its index in the sequence.
type Job = (u32, RgbImage);

/// Encodes frames and writes them to disk on other threads, so the next frame renders meanwhile.
///
/// The queue between them is short, so rendering waits rather than piling frames up in memory.
pub(super) struct Sink {
    jobs: Option<SyncSender<Job>>,
    workers: Vec<JoinHandle<Result<(), Error>>>,
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).context(CreateDirSnafu { path: dir })?;
    }
    fs::write(path, bytes).context(WriteSnafu { path })
}

/// Takes frames off `queue` until it closes, writing each as a still at the path `path_of` gives it.
fn write_stills(
    queue: &Mutex<Receiver<Job>>,
    path_of: &(impl Fn(u32) -> PathBuf + ?Sized),
    encoding: Encoding,
) -> Result<(), Error> {
    loop {
        let job = queue.lock().unwrap_or_else(PoisonError::into_inner).recv();
        let Ok((index, image)) = job else { return Ok(()) };
        let path = path_of(index);
        let bytes = encode(&image, encoding).context(EncodeSnafu { path: &path })?;
        write(&path, &bytes)?;
    }
}

/// Takes frames off `queue` in order until it closes, then writes them to `path` as one animation.
fn write_animation(queue: Receiver<Job>, path: &Path, fps: f32, encoding: Encoding) -> Result<(), Error> {
    let mut animation = None;
    for (_, image) in queue {
        let animation = match &mut animation {
            Some(animation) => animation,
            None => animation
                .insert(Animation::new(image.width(), image.height(), fps, encoding).context(EncodeSnafu { path })?),
        };
        animation.push(&image).context(EncodeSnafu { path })?;
    }
    let Some(animation) = animation else { return Ok(()) };
    write(path, &animation.finish().context(EncodeSnafu { path })?)
}

impl Sink {
    /// Writes each frame as its own file, at the path `path_of` gives its index.
    pub(super) fn stills(path_of: impl Fn(u32) -> PathBuf + Send + Sync + 'static, encoding: Encoding) -> Self {
        let count = thread::available_parallelism().map_or(1, NonZero::get).min(MAX_WORKERS);
        let (jobs, queue) = sync_channel(count);
        let queue = Arc::new(Mutex::new(queue));
        let path_of = Arc::new(path_of);
        let workers = (0..count)
            .map(|_| {
                let (queue, path_of) = (Arc::clone(&queue), Arc::clone(&path_of));
                thread::spawn(move || write_stills(&queue, &*path_of, encoding))
            })
            .collect();
        Self { jobs: Some(jobs), workers }
    }

    /// Writes the frames to `path` as one animated WebP playing at `fps`.
    pub(super) fn animation(path: PathBuf, fps: f32, encoding: Encoding) -> Self {
        let (jobs, queue) = sync_channel(2);
        let worker = thread::spawn(move || write_animation(queue, &path, fps, encoding));
        Self { jobs: Some(jobs), workers: vec![worker] }
    }

    /// Queues a frame; false once the writers have stopped on an error, which `finish` returns.
    pub(super) fn push(&self, index: u32, image: RgbImage) -> bool {
        self.jobs.as_ref().is_some_and(|jobs| jobs.send((index, image)).is_ok())
    }

    /// Waits for every queued frame to be written; the first failure, if any.
    pub(super) fn finish(mut self) -> Result<(), Error> {
        self.jobs = None;
        let mut outcome = Ok(());
        for worker in self.workers.drain(..) {
            let result = worker.join().unwrap_or_else(|payload| panic::resume_unwind(payload));
            outcome = outcome.and(result);
        }
        outcome
    }
}
