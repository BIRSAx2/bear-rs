use std::sync::atomic::{AtomicU8, Ordering};

static VERBOSITY: AtomicU8 = AtomicU8::new(0);

pub fn set(level: u8) {
    VERBOSITY.store(level, Ordering::Relaxed);
}

pub fn level() -> u8 {
    VERBOSITY.load(Ordering::Relaxed)
}

pub fn enabled(level: u8) -> bool {
    self::level() >= level
}

pub fn eprintln(level: u8, message: impl AsRef<str>) {
    if enabled(level) {
        std::eprintln!("{}", message.as_ref());
    }
}
