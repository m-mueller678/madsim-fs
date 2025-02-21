use std::io::{Error, Result, SeekFrom};
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt, ReadBuf,
};

#[cfg(madsim)]
mod fs;

#[cfg(not(madsim))]
use tokio::fs;

pub use fs::OpenOptions;

pin_project_lite::pin_project! {
    /// A reference to an open file on the filesystem.
    pub struct File {
        #[pin]
        inner: fs::File,
    }
}

impl File {
    pub fn options() -> fs::OpenOptions {
        fs::File::options()
    }

    pub async fn open(path: impl AsRef<Path>) -> Result<File> {
        OpenOptions::new().read(true).open(path.as_ref())
    }

    pub fn create(path: impl AsRef<Path>) -> Result<File> {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path.as_ref())
    }

    pub fn create_new(path: impl AsRef<Path>) -> Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path.as_ref())
    }

    pub async fn sync_all(&self) -> Result<()> {
        self.inner.sync_all()
    }

    pub async fn sync_data(&self) -> Result<()> {
        self.inner.sync_data()
    }
}

impl AsyncWrite for File {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        self.project().inner.poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().inner.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().inner.poll_shutdown(cx)
    }
}

impl AsyncRead for File {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().inner.poll_read(cx, buf)
    }
}

impl AsyncSeek for File {
    fn start_seek(self: Pin<&mut Self>, position: SeekFrom) -> std::io::Result<()> {
        self.project().inner.start_seek(position)
    }

    fn poll_complete(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
        self.project().inner.poll_complete(cx)
    }
}
