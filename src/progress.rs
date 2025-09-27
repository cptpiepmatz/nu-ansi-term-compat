use std::{
    borrow::Cow,
    io::{Write, stderr},
    sync::LazyLock,
    time::Duration,
};

use console::{Alignment, pad_str, style};
use indicatif::{HumanDuration, ProgressBar, ProgressStyle};

const PADDING_WIDTH: usize = 12;
const UPDATE_INTERVAL: Duration = Duration::from_millis(100);

const SPINNER_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
    ProgressStyle::with_template("{prefix:>12.cyan.bold} {spinner} {msg}... ({elapsed})")
        .expect("is valid template")
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
});

const BAR_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
    ProgressStyle::with_template("{prefix:>12.cyan.bold} [{bar}] {human_pos}/{human_len}: {msg}... ({eta} remaining)")
        .expect("is valid template")
        .progress_chars("=> ")
});

#[derive(Debug)]
pub struct Progress {
    progress_bar: Option<ProgressBar>,
}

impl Drop for Progress {
    fn drop(&mut self) {
        if let Some(progress_bar) = &self.progress_bar {
            progress_bar.disable_steady_tick();
            progress_bar.finish_and_clear();
        }
    }
}

impl Progress {
    pub fn new() -> Self {
        Self { progress_bar: None }
    }

    pub fn spinner(
        &mut self,
        prefix: impl Into<Cow<'static, str>>,
        msg: impl Into<Cow<'static, str>>,
    ) {
        let spinner = ProgressBar::new_spinner()
            .with_prefix(prefix)
            .with_message(msg)
            .with_style(SPINNER_STYLE.clone());
        spinner.enable_steady_tick(UPDATE_INTERVAL);
        self.progress_bar = Some(spinner);
    }

    pub fn bar(
        &mut self,
        total: usize,
        prefix: impl Into<Cow<'static, str>>,
        msg: impl Into<Cow<'static, str>>,
    ) -> impl Fn() {
        let bar = ProgressBar::new(total as u64)
            .with_prefix(prefix)
            .with_message(msg)
            .with_style(BAR_STYLE.clone());
        bar.enable_steady_tick(UPDATE_INTERVAL);
        self.progress_bar = Some(bar);
        || self.progress_bar.iter().for_each(|pb| pb.inc(1))
    }

    pub fn finish(
        &mut self,
        prefix: impl Into<Cow<'static, str>>,
        msg: impl Into<Cow<'static, str>>,
    ) {
        self.log(console::Color::Green, prefix, msg)
    }

    pub fn warning(
        &mut self,
        msg: impl Into<Cow<'static, str>>
    ) {
        self.log(console::Color::Yellow, "Warning", msg)
    }

    fn log(
        &mut self,
        color: console::Color,
        prefix: impl Into<Cow<'static, str>>,
        msg: impl Into<Cow<'static, str>>,
    ) {
        if let Some(progress_bar) = &self.progress_bar {
            progress_bar.disable_steady_tick();
            progress_bar.finish_and_clear();
        }

        let prefix = prefix.into();
        let prefix = style(pad_str(&prefix, PADDING_WIDTH, Alignment::Right, None))
            .bold()
            .fg(color);

        let mut stderr = stderr();
        let _ = write!(stderr, "{prefix} {}", msg.into());
        if let Some(elapsed) = self.progress_bar.as_ref().map(|pb| pb.elapsed()) {
            let _ = write!(stderr, " in {}", HumanDuration(elapsed));
        }
        let _ = writeln!(stderr);
    }
}
