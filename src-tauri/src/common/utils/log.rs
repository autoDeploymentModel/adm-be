/// Debug log macro: always writes to the daily log file under
/// ~/.local/share/com.adm.be/logs/ (Linux) or ~/.adm-be/logs/ (Windows).
/// In debug builds, also prints to stderr.
///
/// Usage: `dbg_log!("...")` or `dbg_log!("{:?}", val)`
/// 注意：消息内容不要再带 `[DEBUG]` 等级别前缀，宏会自动加。
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
