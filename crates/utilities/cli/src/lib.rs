//! Shared CLI utilities for the Ethgas node: logging setup, version reporting,
//! panic backtraces, and the common clap argument surface.

mod backtrace;
pub use backtrace::Backtracing;

mod logging;
pub use logging::{
    FileLogConfig, LogConfig, LogFormat, LogLevel, LogRotation, StdoutLogConfig,
    verbosity_to_level_filter,
};

mod version;
pub use version::Version;

mod cli;
