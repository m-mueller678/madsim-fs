use futures::FutureExt;
use madsim::export::futures;
use std::io::{Error, IoSlice, Result, SeekFrom};
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
    fn from_inner(inner: Result<fs::File>) -> Result<Self> {
        Ok(Self { inner: inner? })
    }

    pub fn options() -> fs::OpenOptions {
        fs::File::options()
    }

    pub async fn open(path: impl AsRef<Path>) -> Result<File> {
        Self::options()
            .read(true)
            .open(path.as_ref())
            .map(Self::from_inner)
            .await
    }

    pub async fn create(path: impl AsRef<Path>) -> Result<File> {
        Self::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path.as_ref())
            .map(Self::from_inner)
            .await
    }

    pub async fn create_new(path: impl AsRef<Path>) -> Result<File> {
        Self::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path.as_ref())
            .map(Self::from_inner)
            .await
    }

    pub async fn sync_all(&self) -> Result<()> {
        self.inner.sync_all().await
    }

    pub async fn sync_data(&self) -> Result<()> {
        self.inner.sync_data().await
    }
}

impl AsyncWrite for File {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        self.project().inner.poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.project().inner.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.project().inner.poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<std::result::Result<usize, Error>> {
        self.project().inner.poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.project().inner.is_write_vectored()
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
