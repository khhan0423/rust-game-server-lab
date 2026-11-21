//! Owned readers with a bounded window of recent child output.

use std::{collections::VecDeque, io};

use infra::ready::ReadyLine;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    sync::{oneshot, watch},
    task::JoinHandle,
};

const RECENT_LINES: usize = 100;
const LINE_BYTES: usize = 8192;

#[derive(Debug)]
pub(crate) struct Output {
    lines: watch::Receiver<VecDeque<String>>,
    reader: JoinHandle<io::Result<()>>,
}

impl Output {
    pub(crate) fn read(
        pipe: impl AsyncRead + Unpin + Send + 'static,
        ready: Option<oneshot::Sender<ReadyLine>>,
    ) -> Self {
        let (send, lines) = watch::channel(VecDeque::with_capacity(RECENT_LINES));
        let reader = tokio::spawn(async move {
            let mut pipe = BufReader::new(pipe);
            let mut ready = ready;
            let mut bytes = Vec::new();
            let mut truncated = false;
            loop {
                let chunk = pipe.fill_buf().await?;
                let end = chunk.iter().position(|&b| b == b'\n');
                let count = end.unwrap_or(chunk.len());
                let keep = count.min(LINE_BYTES - bytes.len());
                bytes.extend_from_slice(&chunk[..keep]);
                truncated |= keep < count;
                let eof = chunk.is_empty();
                pipe.consume(count + usize::from(end.is_some()));
                if end.is_none() && !eof {
                    continue;
                }
                if end.is_some() || !bytes.is_empty() {
                    let line = String::from_utf8_lossy(&bytes);
                    let line = line.trim_end_matches('\r');
                    if let Some(parsed) = (!truncated)
                        .then(|| line.parse::<ReadyLine>().ok())
                        .flatten()
                        && let Some(ready) = ready.take()
                    {
                        // The owner stopped waiting if the receiver has been closed.
                        let _ = ready.send(parsed);
                    }
                    send.send_modify(|lines| {
                        if lines.len() == RECENT_LINES {
                            lines.pop_front();
                        }
                        lines.push_back(line.to_owned());
                    });
                    bytes.clear();
                    truncated = false;
                }
                if eof {
                    return Ok(());
                }
            }
        });
        Self { lines, reader }
    }

    pub(crate) fn recent(&self) -> Vec<String> {
        self.lines.borrow().iter().cloned().collect()
    }

    pub(crate) async fn finish(&mut self) -> anyhow::Result<()> {
        Ok((&mut self.reader).await??)
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.reader.abort();
    }
}
