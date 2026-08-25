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
    // Linux: ~/.local/share/com.adm.be/（用户目录，有写权限）
    #[cfg(target_os = "linux")]
    {
        if let Some(dir) = dirs::data_local_dir() {
            let data_dir = dir.join("com.adm.be");
            std::fs::create_dir_all(&data_dir)
                .map_err(|e| AppError::msg(format!("创建数据目录失败: {}", e)))?;
            return Ok(data_dir);
        }
    }

    // Windows / 兜底：可执行文件同目录
    let _ = app;
    get_exe_dir()
}
