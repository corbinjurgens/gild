// Fire-and-forget timing log — all write errors are intentionally ignored.
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

pub struct Logger {
    writer: BufWriter<File>,
    start: Instant,
    phase_start: Option<(&'static str, Instant)>,
    finished: bool,
}

impl Logger {
    pub fn open(path: &Path) -> Option<Self> {
        let file = File::create(path).ok()?;
        let mut logger = Self {
            writer: BufWriter::new(file),
            start: Instant::now(),
            phase_start: None,
            finished: false,
        };
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(logger.writer, "gild log started at {now}");
        let _ = writeln!(logger.writer, "version: {}", env!("CARGO_PKG_VERSION"));
        Some(logger)
    }

    pub fn info(&mut self, msg: &str) {
        let elapsed = self.start.elapsed();
        let _ = writeln!(self.writer, "[{:>8.3}s] {}", elapsed.as_secs_f64(), msg);
    }

    pub fn phase_start(&mut self, name: &'static str) {
        self.phase_end();
        self.info(&format!("phase start: {name}"));
        self.phase_start = Some((name, Instant::now()));
    }

    pub fn phase_end(&mut self) {
        if let Some((name, t)) = self.phase_start.take() {
            let dur = t.elapsed();
            let _ = writeln!(
                self.writer,
                "[{:>8.3}s] phase done:  {} ({:.3}s)",
                self.start.elapsed().as_secs_f64(),
                name,
                dur.as_secs_f64(),
            );
        }
    }

    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.phase_end();
        let total = self.start.elapsed();
        let _ = writeln!(
            self.writer,
            "[{:>8.3}s] total elapsed: {:.3}s",
            total.as_secs_f64(),
            total.as_secs_f64()
        );
        let _ = self.writer.flush();
    }
}

impl Drop for Logger {
    fn drop(&mut self) {
        self.finish();
    }
}
