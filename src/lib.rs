// Copyright (C) 2022 Leandro Lisboa Penz <lpenz@lpenz.org>
// This file is subject to the terms and conditions defined in
// file 'LICENSE', which is part of this source code package.

#![deny(future_incompatible)]
#![deny(nonstandard_style)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

//! tokio-process-stream is a simple crate that wraps a [`tokio::process`] into a
//! [`tokio::stream`]
//!
//! Having a stream interface to processes is useful when we have multiple sources of data that
//! we want to merge and start processing from a single entry point.
//!
//! This crate provides a [`futures::stream::Stream`] wrapper for [`tokio::process::Child`]. The
//! main struct is [`ProcessLineStream`], which implements the trait, yielding one [`Item`] enum
//! at a time, each containing one line from either stdout ([`Item::Stdout`]) or stderr
//! ([`Item::Stderr`]) of the underlying process until it exits. At this point, the stream
//! yields a single [`Item::Done`] and finishes.
//!
//! Example usage:
//!
//! ```rust
//! use tokio_process_stream::ProcessLineStream;
//! use tokio::process::Command;
//! use tokio_stream::StreamExt;
//! use std::error::Error;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn Error>> {
//!     let mut sleep_cmd = Command::new("sleep");
//!     sleep_cmd.args(&["1"]);
//!     let ls_cmd = Command::new("ls");
//!
//!     let sleep_procstream = ProcessLineStream::try_from(sleep_cmd)?;
//!     let ls_procstream = ProcessLineStream::try_from(ls_cmd)?;
//!     let mut procstream = sleep_procstream.merge(ls_procstream);
//!
//!     while let Some(item) = procstream.next().await {
//!         println!("{:?}", item);
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! # Streaming chunks
//!
//! It is also possible to stream `Item<Bytes>` chunks with [`ProcessChunkStream`].
//!
//! ```rust
//! use tokio_process_stream::{Item, ProcessChunkStream};
//! use tokio::process::Command;
//! use tokio_stream::StreamExt;
//! use std::error::Error;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn Error>> {
//!     let mut procstream: ProcessChunkStream = Command::new("/bin/sh")
//!         .arg("-c")
//!         .arg(r#"printf "1/2"; sleep 0.1; printf "\r2/2 done\n""#)
//!         .try_into()?;
//!
//!     while let Some(item) = procstream.next().await {
//!         println!("{:?}", item);
//!     }
//!     Ok(())
//! }
//! ```

use pin_project_lite::pin_project;
use std::{
    fmt,
    future::Future,
    io,
    pin::Pin,
    process::{ExitStatus, Stdio},
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, ChildStderr, ChildStdout, Command},
};
use tokio_stream::{wrappers::LinesStream, Stream};
use tokio_util::io::ReaderStream;

/// [`ProcessStream`] output.
#[derive(Debug)]
pub enum Item<Out> {
    /// A stdout chunk printed by the process.
    Stdout(Out),
    /// A stderr chunk printed by the process.
    Stderr(Out),
    /// The [`ExitStatus`](std::process::ExitStatus), yielded after the process exits.
    Done(io::Result<ExitStatus>),
}

impl<T> Item<T>
where
    T: std::ops::Deref,
{
    /// Returns a [`Item::Stdout`] dereference, otherwise `None`.
    pub fn stdout(&self) -> Option<&T::Target> {
        match self {
            Self::Stdout(s) => Some(s),
            _ => None,
        }
    }

    /// Returns a [`Item::Stderr`] dereference, otherwise `None`.
    pub fn stderr(&self) -> Option<&T::Target> {
        match self {
            Self::Stderr(s) => Some(s),
            _ => None,
        }
    }
}

impl<Out: fmt::Display> fmt::Display for Item<Out> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Item::Stdout(s) => fmt::Display::fmt(&s, f),
            Item::Stderr(s) => fmt::Display::fmt(&s, f),
            _ => Ok(()),
        }
    }
}

pin_project! {
/// The main tokio-process-stream struct, which implements the
/// [`Stream`](tokio_stream::Stream) trait
#[derive(Debug)]
pub struct ChildStream<Sout, Serr> {
    child: Option<Child>,
    stdout: Option<Sout>,
    stderr: Option<Serr>,
}
}

impl<Sout, Serr> TryFrom<Command> for ChildStream<Sout, Serr>
where
    ChildStream<Sout, Serr>: From<Child>,
{
    type Error = io::Error;
    fn try_from(mut command: Command) -> io::Result<Self> {
        Self::try_from(&mut command)
    }
}

impl<Sout, Serr> TryFrom<&mut Command> for ChildStream<Sout, Serr>
where
    ChildStream<Sout, Serr>: From<Child>,
{
    type Error = io::Error;
    fn try_from(command: &mut Command) -> io::Result<Self> {
        Ok(command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?
            .into())
    }
}

impl<T, Sout, Serr> Stream for ChildStream<Sout, Serr>
where
    Sout: Stream<Item = io::Result<T>> + std::marker::Unpin,
    Serr: Stream<Item = io::Result<T>> + std::marker::Unpin,
{
    type Item = Item<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.child.is_none() {
            // Keep returning None after we are done and everything is dropped
            return Poll::Ready(None);
        }
        let this = self.project();
        if let Some(stderr) = this.stderr {
            match Pin::new(stderr).poll_next(cx) {
                Poll::Ready(Some(line)) => {
                    return Poll::Ready(Some(Item::Stderr(line.unwrap())));
                }
                Poll::Ready(None) => {
                    *this.stderr = None;
                }
                Poll::Pending => {}
            }
        }
        if let Some(stdout) = this.stdout {
            match Pin::new(stdout).poll_next(cx) {
                Poll::Ready(Some(line)) => {
                    return Poll::Ready(Some(Item::Stdout(line.unwrap())));
                }
                Poll::Ready(None) => {
                    *this.stdout = None;
                }
                Poll::Pending => {}
            }
        }
        if this.stdout.is_none() && this.stderr.is_none() {
            // Streams closed, all that is left is waiting for the child to exit:
            if let Some(mut child) = std::mem::take(&mut *this.child) {
                if let Poll::Ready(sts) = Pin::new(&mut Box::pin(child.wait())).poll(cx) {
                    return Poll::Ready(Some(Item::Done(sts)));
                }
                // Sometimes the process can close stdout+stderr before it's ready to be
                // 'wait'ed. To handle that, we put child back in this:
                *this.child = Some(child);
            }
        }
        Poll::Pending
    }
}

/// [`ChildStream`] that produces lines.
pub type ProcessLineStream =
    ChildStream<LinesStream<BufReader<ChildStdout>>, LinesStream<BufReader<ChildStderr>>>;

/// Alias for [`ProcessLineStream`].
pub type ProcessStream = ProcessLineStream;

impl From<Child> for ProcessLineStream {
    fn from(mut child: Child) -> Self {
        let stdout = child
            .stdout
            .take()
            .map(|s| LinesStream::new(BufReader::new(s).lines()));
        let stderr = child
            .stderr
            .take()
            .map(|s| LinesStream::new(BufReader::new(s).lines()));
        Self {
            child: Some(child),
            stdout,
            stderr,
        }
    }
}

/// [`ChildStream`] that produces chunks that may part of a line or multiple lines.
///
/// # Example
/// ```
/// use tokio_process_stream::{Item, ProcessChunkStream};
/// use tokio::process::Command;
/// use tokio_stream::StreamExt;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Example of a process that prints onto a single line using '\r'.
/// let mut procstream: ProcessChunkStream = Command::new("/bin/sh")
///     .arg("-c")
///     .arg(r#"printf "1/2"; sleep 0.1; printf "\r2/2 done\n""#)
///     .try_into()?;
///
/// assert_eq!(
///     procstream.next().await.as_ref().and_then(|n| n.stdout()),
///     Some(b"1/2" as _)
/// );
/// assert_eq!(
///     procstream.next().await.as_ref().and_then(|n| n.stdout()),
///     Some(b"\r2/2 done\n" as _)
/// );
/// assert!(matches!(procstream.next().await, Some(Item::Done(_))));
/// # Ok(()) }
/// ```
pub type ProcessChunkStream =
    ChildStream<ReaderStream<BufReader<ChildStdout>>, ReaderStream<BufReader<ChildStderr>>>;

impl From<Child> for ProcessChunkStream {
    fn from(mut child: Child) -> Self {
        let stdout = child
            .stdout
            .take()
            .map(|s| ReaderStream::new(BufReader::new(s)));
        let stderr = child
            .stderr
            .take()
            .map(|s| ReaderStream::new(BufReader::new(s)));
        Self {
            child: Some(child),
            stdout,
            stderr,
        }
    }
}
