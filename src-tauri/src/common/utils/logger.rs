use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use chrono::Local;
use crate::common::error::AppError;

const MAX_LOG_DAYS: i64 = 7;
const MAX_READ_LINES: usize = 5000;

pub fn get_log_dir() -> Result<PathBuf, AppError> {
    // Linux: ~/.local/share/com.adm.be/logs/（与多机 vLLM 流水日志同目录，统一管理）
    #[cfg(target_os = "linux")]
    {
        if let Some(dir) = dirs::data_local_dir() {
            let log_dir = dir.join("com.adm.be").join("logs");
            std::fs::create_dir_all(&log_dir)
                .map_err(|e| AppError::msg(format!("创建日志目录失败: {}", e)))?;
            return Ok(log_dir);
        }
    }

    // Windows / 兜底：保持原样 ~/.adm-be/logs/
    let home = dirs::home_dir()
        .ok_or(AppError::msg("无法获取用户目录"))?;
    let log_dir = home.join(".adm-be").join("logs");
    if !log_dir.exists() {
        fs::create_dir_all(&log_dir)
            .map_err(|e| AppError::msg(format!("创建日志目录失败: {}", e)))?;
    }
    Ok(log_dir)
}

fn get_log_path(date: &str) -> Result<PathBuf, AppError> {
    let dir = get_log_dir()?;
    Ok(dir.join(format!("{}.log", date)))
}

pub fn today_str() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn write_log(level: &str, tag: &str, message: &str) {
    if let Ok(path) = get_log_path(&today_str()) {
        let now = Local::now().format("%H:%M:%S%.3f").to_string();
        let line = format!("[{}] [{}] [{}] {}\n", now, level, tag, message);
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

pub fn read_log(date: &str) -> Result<String, AppError> {
    let path = get_log_path(date)?;
    if !path.exists() {
        return Ok(String::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| AppError::msg(format!("读取日志失败: {}", e)))?;
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() > MAX_READ_LINES {
        let start = lines.len() - MAX_READ_LINES;
        Ok(format!("... (仅显示最后 {} 行，共 {} 行)\n{}",
            MAX_READ_LINES, lines.len(), lines[start..].join("\n")))
    } else {
        Ok(content)
    }
}

pub fn list_log_dates() -> Result<Vec<String>, AppError> {
    let dir = get_log_dir()?;
    let mut dates = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".log") {
                    dates.push(name.trim_end_matches(".log").to_string());
                }
            }
        }
    }
    dates.sort();
    dates.reverse();
    Ok(dates)
}

pub fn cleanup_old_logs() {
    let dir = match get_log_dir() { Ok(d) => d, Err(_) => return };
    let cutoff = Local::now().date_naive() - chrono::Duration::days(MAX_LOG_DAYS);
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if !name.ends_with(".log") { continue; }
                let date_str = name.trim_end_matches(".log");
                if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                    if date < cutoff {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
}

pub fn init() {
    cleanup_old_logs();
}

pub fn clear_all_logs() -> Result<(), AppError> {
    let dir = get_log_dir()?;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".log") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }
    Ok(())
}
