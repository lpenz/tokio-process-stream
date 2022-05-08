[![CI](https://github.com/lpenz/tokio-process-stream/actions/workflows/ci.yml/badge.svg)](https://github.com/lpenz/tokio-process-stream/actions/workflows/ci.yml)
[![coveralls](https://coveralls.io/repos/github/lpenz/tokio-process-stream/badge.svg?branch=main)](https://coveralls.io/github/lpenz/tokio-process-stream?branch=main)
[![crates.io](https://img.shields.io/crates/v/tokio-process-stream)](https://crates.io/crates/tokio-process-stream)
[![doc.rs](https://docs.rs/tokio-process-stream/badge.svg)](https://docs.rs/tokio-process-stream)

# tokio-process-stream

tokio-process-stream is a simple crate that wraps a [`tokio::process`] into a
[`tokio::stream`]

Having a stream interface to processes is useful when we have multiple sources of data that
we want to merge and start processing from a single entry point.

This crate provides a [`futures::stream::Stream`] wrapper for [`tokio::process::Child`]. The
main struct is [`ProcessLineStream`], which implements the trait, yielding one [`Item`] enum
at a time, each containing one line from either stdout ([`Item::Stdout`]) or stderr
([`Item::Stderr`]) of the underlying process until it exits. At this point, the stream
yields a single [`Item::Done`] and finishes.

Example usage:

```rust
use tokio_process_stream::ProcessLineStream;
use tokio::process::Command;
use tokio_stream::StreamExt;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut sleep_cmd = Command::new("sleep");
    sleep_cmd.args(&["1"]);
    let ls_cmd = Command::new("ls");

    let sleep_procstream = ProcessLineStream::try_from(sleep_cmd)?;
    let ls_procstream = ProcessLineStream::try_from(ls_cmd)?;
    let mut procstream = sleep_procstream.merge(ls_procstream);

    while let Some(item) = procstream.next().await {
        println!("{:?}", item);
    }

    Ok(())
}
```

## Streaming chunks

It is also possible to stream `Item<Bytes>` chunks with [`ProcessChunkStream`].

```rust
use tokio_process_stream::{Item, ProcessChunkStream};
use tokio::process::Command;
use tokio_stream::StreamExt;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut procstream: ProcessChunkStream = Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"printf "1/2"; sleep 0.1; printf "\r2/2 done\n""#)
        .try_into()?;

    while let Some(item) = procstream.next().await {
        println!("{:?}", item);
    }
    Ok(())
}
```

[`tokio::process`]: https://docs.rs/tokio/latest/tokio/process
[`tokio::stream`]: https://docs.rs/futures-core/latest/futures_core/stream
[`futures::stream::Stream`]: https://docs.rs/futures-core/latest/futures_core/stream/trait.Stream.html
[`tokio::process::Child`]: https://docs.rs/tokio/latest/tokio/process/struct.Child.html
[`ProcessLineStream`]: https://docs.rs/tokio-process-stream/latest/tokio_process_stream/struct.ProcessLineStream.html
[`ProcessChunkStream`]: https://docs.rs/tokio-process-stream/latest/tokio_process_stream/struct.ProcessChunkStream.html
[`Item`]: https://docs.rs/tokio-process-stream/latest/tokio_process_stream/enum.Item.html
[`Item::Stdout`]: https://docs.rs/tokio-process-stream/latest/tokio_process_stream/enum.Item.html#variant.Stdout
[`Item::Stderr`]: https://docs.rs/tokio-process-stream/latest/tokio_process_stream/enum.Item.html#variant.Stderr
[`Item::Done`]: https://docs.rs/tokio-process-stream/latest/tokio_process_stream/enum.Item.html#variant.Done
