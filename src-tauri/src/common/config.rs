use std::path::PathBuf;
use crate::common::error::AppError;

pub fn get_exe_dir() -> Result<PathBuf, AppError> {
    std::env::current_exe()
        ?
        .parent()
        .ok_or(AppError::msg("无法获取可执行文件目录"))
        .map(|p| p.to_path_buf())
}

pub fn get_data_dir(app: Option<&tauri::AppHandle>) -> Result<PathBuf, AppError> {
    let _ = app;
    get_exe_dir()
}

pub fn get_base_dir(app: Option<&tauri::AppHandle>) -> Result<PathBuf, AppError> {
    let _ = app;
    get_exe_dir()
}
