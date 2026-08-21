/// Debug log macro: always writes to the daily log file under ~/.adm-be/logs/.
/// In debug builds, also prints to stderr.
///
/// Usage: `dbg_log!("...")` or `dbg_log!("{:?}", val)`
#[macro_export]
macro_rules! dbg_log {
    ($($arg:tt)*) => {
        {
            let msg = format!($($arg)*);
            #[cfg(debug_assertions)]
            eprintln!("{}", msg);
            $crate::common::utils::logger::write_log("DEBUG", "APP", &msg);
        }
    };
}

/// Application log macro with explicit level and tag.
/// Usage: `app_log!("INFO", "MODEL", "Starting model {}", id)`
#[macro_export]
macro_rules! app_log {
    ($level:expr, $tag:expr, $($arg:tt)*) => {
        {
            let msg = format!($($arg)*);
            $crate::common::utils::logger::write_log($level, $tag, &msg);
        }
    };
}
