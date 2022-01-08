// Copyright (C) 2022 Leandro Lisboa Penz <lpenz@lpenz.org>
// This file is subject to the terms and conditions defined in
// file 'LICENSE', which is part of this source code package.

use tokio_process_stream::*;

use anyhow::anyhow;
use anyhow::Result;
use std::convert::TryFrom;
use std::process::Stdio;
use tokio::process::Command;
use tokio_stream::StreamExt;

fn none2err<T>(opt: Option<T>) -> Result<T> {
    opt.ok_or_else(|| anyhow!("unexpected None"))
}

#[tokio::test]
async fn basicout() -> Result<()> {
    let mut cmd = Command::new("/bin/sh");
    cmd.args(&["-c", "printf 'test1\ntest2'"]);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let child = cmd.spawn()?;
    let mut procstream = ProcessStream::from(child);
    assert_eq!(none2err(procstream.next().await)?.stdout(), Some("test1"));
    assert_eq!(none2err(procstream.next().await)?.stdout(), Some("test2"));
    let exitstatus = procstream.next().await;
    if let Some(Item::Done(sts)) = exitstatus {
        assert!(sts?.success());
    } else {
        panic!("invalid exit status {:?}", exitstatus);
    }
    assert!(procstream.next().await.is_none());
    Ok(())
}

#[tokio::test]
async fn basicerr() -> Result<()> {
    let mut cmd = Command::new("/bin/sh");
    cmd.args(&["-c", "printf 'test1\ntest2' >&2"]);
    let mut procstream = ProcessStream::try_from(cmd)?;
    assert_eq!(none2err(procstream.next().await)?.stderr(), Some("test1"));
    assert_eq!(none2err(procstream.next().await)?.stderr(), Some("test2"));
    let exitstatus = procstream.next().await;
    if let Some(Item::Done(sts)) = exitstatus {
        assert!(sts?.success());
    } else {
        panic!("invalid exit status {:?}", exitstatus);
    }
    assert!(procstream.next().await.is_none());
    Ok(())
}

#[tokio::test]
async fn close_stds() -> Result<()> {
    let mut cmd = Command::new("/bin/sh");
    cmd.args(&["-c", "exec true 1>&- 2>&-"]);
    let mut procstream = ProcessStream::try_from(cmd)?;
    let exitstatus = procstream.next().await;
    if let Some(Item::Done(sts)) = exitstatus {
        assert!(sts?.success());
    } else {
        panic!("invalid exit status {:?}", exitstatus);
    }
    assert!(procstream.next().await.is_none());
    Ok(())
}
