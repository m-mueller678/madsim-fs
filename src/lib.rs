use atomic_refcell::AtomicRefCell;
use madsim::plugin::{node, simulator, Simulator};
use madsim::rand::GlobalRng;
use madsim::task::{NodeId, Spawner};
use madsim::time::{Sleep, TimeHandle};
use madsim::Config;
use snap_buf::SnapBuf;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io::{Error, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{ready, Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncSeek, AsyncWrite};

mod file_contents;

#[cfg(madsim)]
struct FsSimulator {
    file_systems: Mutex<HashMap<NodeId, Filesystem>>,
    config: Arc<AtomicRefCell<FsConfig>>,
}

struct FsConfig {
    write_delay: Duration,
    flush_delay: Duration,
    allow_dirty_write: bool,
}

impl FsConfig {
    fn flush_sleep(&self, write_time: Instant) -> Option<Sleep> {
        let flush_time = TimeHandle::current().now_instant();
        let until = (flush_time + self.flush_delay).max(write_time + self.write_delay);
        if until == flush_time {
            None
        } else {
            Some(madsim::time::sleep_until(until))
        }
    }
}

#[derive(Clone)]
pub struct FileHistory {
    history: Arc<AtomicRefCell<FileState>>,
}

enum FileState {
    Clean(SnapBuf),
    Written {
        old: SnapBuf,
        new: SnapBuf,
        time: Instant,
    },
    Flush {
        old: SnapBuf,
        new: SnapBuf,
        sleep: madsim::time::Sleep,
    },
}

impl FileState {
    fn new(&self) -> &SnapBuf {
        match self {
            FileState::Clean(new)
            | FileState::Written { new, .. }
            | FileState::Flush { new, .. } => new,
        }
    }

    fn poll_flush(
        &mut self,
        cx: &mut Context<'_>,
        config: &AtomicRefCell<FsConfig>,
    ) -> Poll<Result<(), Error>> {
        loop {
            match self {
                FileState::Clean(x) => return Poll::Ready(Ok(())),
                FileState::Written { old, new, time } => {
                    if let Some(sleep) = config.borrow().flush_sleep(*time) {
                        *self = FileState::Flush {
                            old: old.clone(),
                            new: new.clone(),
                            sleep,
                        }
                    } else {
                        *self = FileState::Clean(new.clone())
                    }
                }
                FileState::Flush { old, new, sleep } => {
                    ready!(Pin::new(sleep).poll(cx));
                    *self = FileState::Clean(new.clone());
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }
}

struct FileHistoryEntry {
    version: u64,
    written: Instant,
    file: FileSnapshot,
}

struct FileSnapshot {
    content: SnapBuf,
}

pub struct Filesystem {
    files: HashMap<PathBuf, Arc<AtomicRefCell<FileHistory>>>,
}

impl Filesystem {
    fn reset(&mut self) {}
}

macro_rules! define_open_options {
    ($($name:ident),*) => {
        pub struct OpenOptions{
            $($name:bool),*
        }

        impl OpenOptions{
            $(pub fn $name(&mut self,$name:bool)-> &mut Self{
                self.$name = $name;
                self
            })*
        }
    };
}

define_open_options! {read,write,append,truncate,create,create_new}

impl OpenOptions {
    async fn open(&self, p: impl AsRef<Path>) -> Result<FileHistory, std::io::Error> {
        todo!()
    }
}

impl Simulator for FsSimulator {
    fn new(rand: &GlobalRng, time: &TimeHandle, config: &Config) -> Self
    where
        Self: Sized,
    {
        Self {
            file_systems: Mutex::new(Default::default()),
            config: Arc::new(AtomicRefCell::new(FsConfig {
                write_delay: Duration::ZERO,
                flush_delay: Duration::ZERO,
                allow_dirty_write: true,
            })),
        }
    }

    fn create_node(&self, id: NodeId) {
        let mut file_systems = self.file_systems.lock().unwrap();
        file_systems.insert(
            id,
            Filesystem {
                files: Default::default(),
            },
        );
    }

    fn reset_node(&self, id: NodeId) {
        let mut file_systems = self
            .file_systems
            .lock()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .reset();
    }
}

pub struct FileHandle {
    pendig_seek: Option<i64>,
    cursor: usize,
    history: FileHistory,
    config: Arc<AtomicRefCell<FsConfig>>,
}

impl AsyncSeek for FileHandle {
    fn start_seek(mut self: Pin<&mut Self>, pos: SeekFrom) -> std::io::Result<()> {
        if self.pendig_seek.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "already started",
            ));
        }
        let new = match pos {
            SeekFrom::Start(x) => x as i64,
            SeekFrom::End(x) => self.history.history.borrow().new().len() as i64 + x,
            SeekFrom::Current(x) => self.cursor as i64 + x,
        };
        self.pendig_seek = Some(new);
        Ok(())
    }

    fn poll_complete(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<u64>> {
        let new = self.pendig_seek.take().unwrap();
        Poll::Ready(if new < 0 {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before file start",
            ))
        } else {
            self.cursor = new as usize;
            Ok(self.cursor as u64)
        })
    }
}

impl AsyncWrite for FileHandle {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        let now = TimeHandle::current().now_instant();
        loop {
            let mut this = self.history.history.borrow_mut();
            match &mut *this {
                FileState::Clean(x) => {
                    let mut new = x.clone();
                    new.write(self.cursor, buf);
                    *this = FileState::Written {
                        old: x.clone(),
                        new,
                        time: now,
                    };
                    break;
                }
                FileState::Written { old, new, time } => {
                    if self.config.borrow().allow_dirty_write {
                        new.write(self.cursor, buf);
                        *time = now;
                        break;
                    } else {
                        drop(this);
                        ready!(self.as_mut().poll_flush(cx));
                    }
                }
                FileState::Flush { old, new, sleep } => {
                    *this = FileState::Written {
                        old: old.clone(),
                        new: new.clone(),
                        time: now,
                    }
                }
            }
        }
        self.cursor += buf.len();
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.history
            .history
            .borrow_mut()
            .poll_flush(cx, &*self.config)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.poll_flush(cx)
    }
}
