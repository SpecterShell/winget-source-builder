use std::io::{IsTerminal, stderr};

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

#[derive(Debug, Clone)]
pub(crate) struct ProgressReporter {
    enabled: bool,
}

impl ProgressReporter {
    pub(crate) fn new() -> Self {
        Self {
            enabled: stderr().is_terminal(),
        }
    }

    pub(crate) fn bar(&self, length: usize, message: impl Into<String>) -> Option<ProgressBar> {
        if !self.enabled {
            return None;
        }

        let progress = ProgressBar::with_draw_target(
            Some(length as u64),
            ProgressDrawTarget::stderr_with_hz(10),
        );
        progress.set_style(
            ProgressStyle::with_template(
                "{msg} [{elapsed_precise}] [{wide_bar}] {pos}/{len} ({percent}%)",
            )
            .expect("invalid progress bar template")
            .progress_chars("=>-"),
        );
        progress.set_message(message.into());
        Some(progress)
    }

    pub(crate) fn spinner(&self, message: impl Into<String>) -> Option<ProgressBar> {
        if !self.enabled {
            return None;
        }

        let progress = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr_with_hz(10));
        progress.set_style(
            ProgressStyle::with_template("{spinner} {msg} [{elapsed_precise}]")
                .expect("invalid spinner template")
                .tick_strings(&["-", "\\", "|", "/"]),
        );
        progress.set_message(message.into());
        progress.enable_steady_tick(std::time::Duration::from_millis(100));
        Some(progress)
    }

    pub(crate) fn inc(progress: &Option<ProgressBar>, delta: u64) {
        if let Some(progress) = progress {
            progress.inc(delta);
        }
    }

    pub(crate) fn finish(progress: Option<ProgressBar>) {
        if let Some(progress) = progress {
            progress.finish_and_clear();
        }
    }

    pub(crate) fn warn(&self, message: impl Into<String>) {
        log::warn!("{}", message.into());
    }
}
