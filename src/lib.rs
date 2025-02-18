extern crate core;

use atomic_refcell::AtomicRefCell;
use madsim::plugin::{node, simulator, Simulator};
use madsim::rand::GlobalRng;
use madsim::task::{NodeId, Spawner};
use madsim::time::{Sleep, TimeHandle};
use madsim::Config;
use snap_buf::SnapBuf;
use std::collections::{hash_map, HashMap, VecDeque};
use std::future::Future;
use std::io::{Error, ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf};

mod file_contents;

#[cfg(madsim)]
struct FsSimulator {
    file_systems: AtomicRefCell<HashMap<NodeId, Arc<Filesystem>>>,
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
    files: AtomicRefCell<HashMap<PathBuf, FileHistory>>,
    config: AtomicRefCell<FsConfig>,
}

struct FsConfig {
    write_delay: Duration,
    flush_delay: Duration,
    allow_dirty_write: bool,
}

impl FsConfig {
    fn new() -> Self {
        FsConfig {
            write_delay: Duration::ZERO,
            flush_delay: Duration::ZERO,
            allow_dirty_write: true,
        }
    }
}

impl Filesystem {
    fn reset(&self) {}
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
    async fn open(&self, p: impl AsRef<Path>) -> Result<FileHandle, Error> {
        let sim = simulator::<FsSimulator>();
        let node = node();
        let fs = sim.file_systems.borrow_mut().get(&node).unwrap().clone();
        let mut files = fs.files.borrow_mut();
        let file = match files.entry(p.as_ref().to_owned()) {
            hash_map::Entry::Occupied(mut x) => {
                if self.create_new {
                    return Err(Error::new(ErrorKind::AlreadyExists, "File already exists"));
                }
                x.into_mut()
            }
            hash_map::Entry::Vacant(x) => {
                if self.create || self.create_new {
                    x.insert(FileHistory {
                        history: Arc::new(AtomicRefCell::new(FileState::Clean(SnapBuf::new()))),
                    })
                } else {
                    return Err(Error::new(ErrorKind::NotFound, "file not found"));
                }
            }
        };
        let fh = FileHandle {
            allow_read: self.read,
            allow_write: self.write || self.append,
            append: self.append,
            pending_seek: None,
            cursor: 0,
            history: file.clone(),
            fs: fs.clone(),
        };
        if self.truncate {
            *fh.history.history.borrow_mut() = FileState::Clean(SnapBuf::new());
        }
        Ok(fh)
    }
}

impl Simulator for FsSimulator {
    fn new(rand: &GlobalRng, time: &TimeHandle, config: &Config) -> Self
    where
        Self: Sized,
    {
        Self {
            file_systems: AtomicRefCell::new(Default::default()),
        }
    }

    fn create_node(&self, id: NodeId) {
        let mut file_systems = self.file_systems.borrow_mut();
        file_systems.insert(
            id,
            Arc::new(Filesystem {
                files: Default::default(),
                config: AtomicRefCell::new(FsConfig::new()),
            }),
        );
    }

    fn reset_node(&self, id: NodeId) {
        let mut file_systems = self.file_systems.borrow_mut().get_mut(&id).unwrap().reset();
    }
}

pub struct FileHandle {
    allow_write: bool,
    allow_read: bool,
    append: bool,
    pending_seek: Option<i64>,
    cursor: usize,
    history: FileHistory,
    fs: Arc<Filesystem>,
}

impl AsyncSeek for FileHandle {
    fn start_seek(mut self: Pin<&mut Self>, pos: SeekFrom) -> std::io::Result<()> {
        if self.pending_seek.is_some() {
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
        self.pending_seek = Some(new);
        Ok(())
    }

    fn poll_complete(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<u64>> {
        let new = self.pending_seek.take().unwrap();
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
        let this = Pin::into_inner(self);
        if !this.allow_write {
            return Poll::Ready(Err(Error::new(
                ErrorKind::InvalidInput,
                "file handle is readonly",
            )));
        }
        let now = TimeHandle::current().now_instant();
        let mut state = this.history.history.borrow_mut();
        loop {
            match &mut *state {
                FileState::Clean(x) => {
                    let mut new = x.clone();
                    if this.append {
                        this.cursor = new.len();
                    }
                    new.write(this.cursor, buf);
                    *state = FileState::Written {
                        old: x.clone(),
                        new,
                        time: now,
                    };
                    break;
                }
                FileState::Written { old, new, time } => {
                    if this.fs.config.borrow().allow_dirty_write {
                        if this.append {
                            this.cursor = new.len();
                        }
                        new.write(this.cursor, buf);
                        *time = now;
                        break;
                    } else {
                        ready!(state.poll_flush(cx, &this.fs.config));
                    }
                }
                FileState::Flush { old, new, sleep } => {
                    *state = FileState::Written {
                        old: old.clone(),
                        new: new.clone(),
                        time: now,
                    }
                }
            }
        }
        this.cursor += buf.len();
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        if !self.allow_write {
            return Poll::Ready(Err(Error::new(
                ErrorKind::InvalidInput,
                "file handle does not allow writes",
            )));
        }
        self.history
            .history
            .borrow_mut()
            .poll_flush(cx, &self.fs.config)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.poll_flush(cx)
    }
}

impl AsyncRead for FileHandle {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = Pin::into_inner(self);
        if !this.allow_read {
            return Poll::Ready(Err(Error::new(
                ErrorKind::InvalidInput,
                "file handle does not allow reads",
            )));
        }
        let file = this.history.history.borrow();
        let data = file.new().read(this.cursor);
        let read_len = buf.remaining().min(data.len());
        this.cursor += read_len;
        buf.put_slice(&data[..read_len]);
        Poll::Ready(Ok(()))
    }
}
