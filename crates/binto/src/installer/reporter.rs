//! Callbacks the install pipeline drives as it works, and the choices it asks the caller to
//! make. The installer only calls these traits; drawing bars and showing prompts is the CLI's
//! business (see `crate::reporter`).

use anyhow::Result;

/// Progress callbacks for downloads, modelled on uv's `Reporter`. A download calls
/// [`on_download_start`](Self::on_download_start) once, feeds the returned id to every later
/// call, and always ends with [`on_download_complete`](Self::on_download_complete) — use
/// [`DownloadProgress`] to get that last call on failure and cancellation too.
pub trait Reporter: Send + Sync {
    /// A download of `size` bytes (when known) is starting. Returns an id for the calls below.
    fn on_download_start(&self, name: &str, size: Option<u64>) -> usize;
    /// `bytes` more bytes arrived for download `id`.
    fn on_download_progress(&self, id: usize, bytes: u64);
    /// Download `id` finished, failed or was dropped.
    fn on_download_complete(&self, id: usize);
}

/// Reports nothing. For tests and callers that have no terminal to draw on.
impl Reporter for () {
    fn on_download_start(&self, _name: &str, _size: Option<u64>) -> usize {
        0
    }
    fn on_download_progress(&self, _id: usize, _bytes: u64) {}
    fn on_download_complete(&self, _id: usize) {}
}

/// One download's progress, from start to completion. Dropping it reports completion, so a
/// download that errors out or whose future is dropped (Ctrl-C) still clears its bar.
pub struct DownloadProgress<'a> {
    reporter: &'a dyn Reporter,
    id: usize,
}

impl<'a> DownloadProgress<'a> {
    pub fn start(reporter: &'a dyn Reporter, name: &str, size: Option<u64>) -> Self {
        let id = reporter.on_download_start(name, size);
        Self { reporter, id }
    }

    pub fn inc(&self, bytes: u64) {
        self.reporter.on_download_progress(self.id, bytes);
    }
}

impl Drop for DownloadProgress<'_> {
    fn drop(&mut self) {
        self.reporter.on_download_complete(self.id);
    }
}

/// The interactive choices the pipeline needs made. A non-interactive implementation (`--yes`)
/// takes the first item and confirms everything.
pub trait Prompter {
    /// Whether a person is answering. Callers that would otherwise guess use this to prefer a
    /// safer default when nobody is.
    fn interactive(&self) -> bool;
    /// Pick one of `items`; returns its index.
    fn select(&self, prompt: &str, items: &[String]) -> Result<usize>;
    /// Pick one of `items` with type-to-filter; returns its index.
    fn fuzzy_select(&self, prompt: &str, items: &[String]) -> Result<usize>;
    /// A yes/no question.
    fn confirm(&self, prompt: &str, default: bool) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl Reporter for Recorder {
        fn on_download_start(&self, name: &str, size: Option<u64>) -> usize {
            self.0
                .lock()
                .unwrap()
                .push(format!("start {name} {size:?}"));
            7
        }
        fn on_download_progress(&self, id: usize, bytes: u64) {
            self.0
                .lock()
                .unwrap()
                .push(format!("progress {id} {bytes}"));
        }
        fn on_download_complete(&self, id: usize) {
            self.0.lock().unwrap().push(format!("complete {id}"));
        }
    }

    #[test]
    fn dropping_the_progress_reports_completion() {
        let recorder = Recorder::default();
        {
            let progress = DownloadProgress::start(&recorder, "fd", Some(10));
            progress.inc(4);
            progress.inc(6);
        }
        assert_eq!(
            *recorder.0.lock().unwrap(),
            [
                "start fd Some(10)",
                "progress 7 4",
                "progress 7 6",
                "complete 7"
            ]
        );
    }
}
