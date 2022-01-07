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
//! This crate provides a [`tokio_stream::Stream`] wrapper for [`tokio::process::Child`].  The
//! main struct is [`ProcessStream`], which implements the trait, yielding one [`Item`] enum at
//! a time, each containing one line from either stdout ([`Item::Stdout`]) or stderr
//! ([`Item::Stderr`]) of the underlying process until it exits. At this point, the stream
//! yields a single [`Item::Done`] and finishes.
//!
//! Example usage:
//!
//! ```rust
//! use tokio_process_stream::ProcessStream;
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
//!     let sleep_procstream = ProcessStream::try_from(sleep_cmd)?;
//!     let ls_procstream = ProcessStream::try_from(ls_cmd)?;
//!     let mut procstream = sleep_procstream.merge(ls_procstream);
//!
//!     while let Some(item) = procstream.next().await {
//!         println!("{:?}", item);
//!     }
//!
//!     Ok(())
//! }
//! ```

use pin_project_lite::pin_project;
use std::convert;
use std::fmt;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::process::ExitStatus;
use std::process::Stdio;
use std::task::Context;
use std::task::Poll;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::Command;
use tokio::process::{ChildStderr, ChildStdout};
use tokio_stream::wrappers::LinesStream;
use tokio_stream::Stream;

/// [`ProcessStream`] yields a stream of `Items`.
#[derive(Debug, PartialEq, Eq)]
pub enum Item {
    /// A stdout line printed by the process.
    Stdout(String),
    /// A stderr line printed by the process.
    Stderr(String),
    /// The [`ExitStatus`](std::process::ExitStatus), yielded after the process exits.
    Done(ExitStatus),
}

impl fmt::Display for Item {
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
pub struct ProcessStream {
    child: Option<Child>,
    stdout: Option<LinesStream<BufReader<ChildStdout>>>,
    stderr: Option<LinesStream<BufReader<ChildStderr>>>,
}
}

impl convert::From<Child> for ProcessStream {
    fn from(mut child: Child) -> ProcessStream {
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

impl convert::TryFrom<Command> for ProcessStream {
    type Error = io::Error;
    fn try_from(mut command: Command) -> io::Result<ProcessStream> {
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let child = command.spawn()?;
        Ok(Self::from(child))
    }
}

impl Stream for ProcessStream {
    type Item = Item;

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
                    return Poll::Ready(Some(Item::Done(sts.unwrap())));
                }
                // Sometimes the process can close stdout+stderr before it's ready to be
                // 'wait'ed. To handle that, we put child back in this:
                *this.child = Some(child);
            }
        }
        Poll::Pending
    }
}
