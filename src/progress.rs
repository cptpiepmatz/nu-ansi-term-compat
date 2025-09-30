use std::{
    borrow::Cow,
    io::{Write, stderr},
    sync::LazyLock,
    time::Duration,
};

use console::{Alignment, pad_str, style};
use human_repr::HumanDuration;
use indicatif::{ProgressBar, ProgressStyle};

const PADDING_WIDTH: usize = 12;
const UPDATE_INTERVAL: Duration = Duration::from_millis(100);

const SPINNER_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
    ProgressStyle::with_template("{prefix:>12.cyan.bold} {spinner} {msg}... ({elapsed})")
        .expect("is valid template")
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
});

const BAR_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
    ProgressStyle::with_template(
        "{prefix:>12.cyan.bold} [{bar}] {human_pos}/{human_len}: {msg}... ({eta} remaining)",
    )
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
    ) -> (impl Fn(), impl Fn(String)) {
        let bar = ProgressBar::new(total as u64)
            .with_prefix(prefix)
            .with_message(msg)
            .with_style(BAR_STYLE.clone());
        bar.enable_steady_tick(UPDATE_INTERVAL);
        self.progress_bar = Some(bar);
        (
            || self.progress_bar.iter().for_each(|pb| pb.inc(1)),
            |msg| {
                let Some(pb) = &self.progress_bar else { return };
                pb.suspend(|| self.println(console::Color::Yellow, "Warning", msg, None));
            },
        )
    }

    pub fn finish(
        &mut self,
        prefix: impl Into<Cow<'static, str>>,
        msg: impl Into<Cow<'static, str>>,
    ) {
        self.log(console::Color::Green, prefix, msg)
    }

    pub fn finish_warning(&mut self, msg: impl Into<Cow<'static, str>>) {
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

        let elapsed = self.progress_bar.as_ref().map(|pb| pb.elapsed());
        self.println(color, prefix, msg, elapsed);
    }

    fn println(
        &self,
        color: console::Color,
        prefix: impl Into<Cow<'static, str>>,
        msg: impl Into<Cow<'static, str>>,
        elapsed: Option<Duration>,
    ) {
        let prefix = prefix.into();
        let prefix = style(pad_str(&prefix, PADDING_WIDTH, Alignment::Right, None))
            .bold()
            .fg(color);

        let mut stderr = stderr();
        let _ = write!(stderr, "{prefix} {}", msg.into());
        if let Some(elapsed) = elapsed {
            let _ = write!(stderr, " in {}", elapsed.human_duration());
        }
        let _ = writeln!(stderr);
    }

    pub fn gix(
        &mut self,
        prefix: impl Into<Cow<'static, str>>,
        msg: impl Into<Cow<'static, str>>,
    ) -> GixProgress<'_> {
        GixProgress {
            progress: self,
            prefix: prefix.into(),
            msg: msg.into(),
        }
    }
}

pub struct GixProgress<'p> {
    progress: &'p mut Progress,
    prefix: Cow<'static, str>,
    msg: Cow<'static, str>,
}

impl<'p> gix::Progress for GixProgress<'p> {
    fn init(
        &mut self,
        max: Option<gix::progress::prodash::progress::Step>,
        unit: Option<gix::progress::Unit>,
    ) {
        match max {
            Some(max) => {
                self.progress
                    .bar(max, self.prefix.clone(), self.msg.clone());
            }
            None => self.progress.spinner(self.prefix.clone(), self.msg.clone()),
        }
    }

    fn set_name(&mut self, name: String) {
        if let Some(pb) = &self.progress.progress_bar {
            pb.set_message(format!("{} ({name})", self.msg));
        }
    }

    fn name(&self) -> Option<String> {
        self.progress.progress_bar.as_ref().map(|pb| pb.message())
    }

    fn id(&self) -> gix::progress::Id {
        gix::progress::UNKNOWN
    }

    fn message(&self, level: gix::progress::MessageLevel, message: String) {
        let Some(pb) = &self.progress.progress_bar else {
            return;
        };
        let (color, prefix) = match level {
            gix::progress::MessageLevel::Info => (console::Color::Blue, "Info"),
            gix::progress::MessageLevel::Failure => (console::Color::Red, "Failure"),
            gix::progress::MessageLevel::Success => (console::Color::Green, "Success"),
        };
        pb.suspend(|| self.progress.println(color, prefix, message, None));
    }
}

impl<'p> gix::Count for GixProgress<'p> {
    fn set(&self, step: gix::progress::prodash::progress::Step) {
        if let Some(pb) = &self.progress.progress_bar {
            pb.set_position(step as u64);
        }
    }

    fn step(&self) -> gix::progress::prodash::progress::Step {
        match &self.progress.progress_bar {
            Some(pb) => pb.position() as usize,
            None => 0,
        }
    }

    fn inc_by(&self, step: gix::progress::prodash::progress::Step) {
        if let Some(pb) = &self.progress.progress_bar {
            pb.inc(step as u64);
        }
    }

    fn counter(&self) -> gix::progress::StepShared {
        unimplemented!("the internal position is not exposed")
    }
}
