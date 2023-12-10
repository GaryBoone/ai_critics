use color_eyre::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

const STARTING_MAX: u64 = 50;

#[derive(Debug, Clone)]
pub struct DoublingProgressBar {
    progress_bar: ProgressBar,
    current_progress: u64,
    max_value: u64,
}

// The DoublingProgressBar struct is a progress bar for open-ended tasks. Instead of progressing
// toward a known, fixed, maximum value, the progress bar will progress toward a maximum value that
// is twice the current value. This allows the progress bar to be used for tasks that have an
// unknown number of steps. The progress bar will start with a maximum value of STARTING_MAX. The
// effect of the doubling is that each time it reaches the current end of the bar, it drops back to
// the halfway point and then continues to grow at half the speed as it did previously.
impl DoublingProgressBar {
    pub fn new(name: &str) -> Result<Self> {
        let progress_bar = ProgressBar::new(STARTING_MAX);
        Self::initialize(progress_bar, name)
    }

    pub fn new_multi(multi_progress: &MultiProgress, name: &str) -> Result<Self> {
        let progress_bar = multi_progress.add(ProgressBar::new(STARTING_MAX));
        Self::initialize(progress_bar, name)
    }

    fn initialize(progress_bar: ProgressBar, name: &str) -> Result<Self> {
        let template = format!(
            "{{spinner:.green}} {} [{{elapsed_precise}}] [{{wide_bar:.cyan/blue}}] {{pos}} chunks received",
            name
        );
        progress_bar.set_style(
            ProgressStyle::default_bar()
                .template(&template)?
                .progress_chars("=▷-"),
        );

        Ok(DoublingProgressBar {
            progress_bar,
            current_progress: 0,
            max_value: STARTING_MAX,
        })
    }

    // Increment the progress, doubling the max value if needed.
    pub fn inc(&mut self) {
        self.current_progress += 1;
        self.progress_bar.inc(1);

        if self.current_progress >= self.max_value {
            self.max_value *= 2;
            self.progress_bar.set_length(self.max_value);
        }
    }

    // Decrement the progress, halving the max value if needed.
    pub fn dec(&mut self) {
        if self.current_progress == 0 {
            return;
        }
        self.current_progress -= 1;
        self.progress_bar.set_position(self.current_progress);

        if self.current_progress <= self.max_value / 2 {
            self.max_value /= 2;
            self.max_value = self.max_value.max(1);
            self.progress_bar.set_length(self.max_value);
        }
    }

    pub fn reset_to_zero(&mut self) {
        self.progress_bar.reset();
        self.current_progress = 0;
        self.max_value = STARTING_MAX;
    }
    pub fn println(self, message: &str) {
        self.progress_bar.println(message);
    }
}
