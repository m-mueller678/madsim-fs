use atomic_refcell::AtomicRefCell;
use madsim::plugin::{node, simulator, Simulator};
use madsim::rand::GlobalRng;
use madsim::task::{NodeId, Spawner};
use madsim::time::TimeHandle;
use madsim::Config;
use snap_buf::SnapBuf;
use std::collections::{HashMap, VecDeque};
use std::io::{Error, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Instant;
use tokio::io::{AsyncSeek, AsyncWrite};

mod file_contents;

#[cfg(madsim)]
struct FsSimulator {
    file_systems: Mutex<HashMap<NodeId, Filesystem>>,
}

#[derive(Clone)]
pub struct FileHistory {
    history: Arc<AtomicRefCell<VecDeque<FileHistoryEntry>>>,
}

impl FileHistory {
    fn update<R>(&self, f: impl FnOnce(&FileSnapshot, u64) -> (Option<FileSnapshot>, R)) -> R {
        let mut history = self.history.borrow_mut();
        let back = history.back().unwrap();
        let new_version = back.version + 1;
        let (new_file, ret) = f(&back.file, new_version);
        history.push_back(FileHistoryEntry {
            version: new_version,
            written: TimeHandle::current().now_instant(),
            file: new_file,
        });
        ret
    }

    fn history_len(&self) -> usize {
        self.history.borrow().len()
    }

    fn inspect<R>(&self, f: impl FnOnce(&FileSnapshot) -> R) -> R {
        f(&self.history.borrow().back().unwrap().file)
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
}

impl AsyncSeek for FileHandle {
    fn start_seek(self: Pin<&mut Self>, pos: SeekFrom) -> std::io::Result<()> {
        if self.pendig_seek.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "already started",
            ));
        }
        let new = match pos {
            SeekFrom::Start(x) => x as i64,
            SeekFrom::End(x) => self.history.inspect(|f| f.content.len()) as i64 + x,
            SeekFrom::Current(x) => self.cursor as i64 + x,
        };
        self.pendig_seek = Some(new);
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
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
    fn flush(&mut self) -> std::io::Result<()> {}

    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        if self.history.history_len() > 3 {
            return Poll::Pending;
        }
        self.history.update(|f, _| {
            let mut content = f.content.clone();
            content.write(self.cursor, buf);
            let f = FileSnapshot { content };
            (f, ())
        });
        Ok(buf.len())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        todo!()
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        todo!()
    }
}
