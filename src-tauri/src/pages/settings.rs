// settings.html 对应逻辑（配置管理）

use crate::common::*;
use crate::common::config;
use crate::dbg_log;
use crate::app_state::AppState;
use tauri::Manager;

// ===== Tauri Command =====

#[tauri::command]
pub async fn save_settings(app: tauri::AppHandle, settings: Settings) -> Result<(), AppError> {
    dbg_log!("[DEBUG] save_settings called with: {:?}", settings);
    // 持有 config 写锁：防止 read-modify-write 并发互相覆盖
    let state = app.state::<AppState>();
    let _lock = state.config_write_lock.lock().map_err(|e| e.to_string())?;
    let data_dir = config::get_data_dir(Some(&app))?;
    let config_path = data_dir.join("config.json");

    let json = serde_json::to_string_pretty(&settings).map_err(|e| AppError::msg(format!("序列化配置失败: {}", e)))?;
    dbg_log!("[DEBUG] Writing config.json to: {:?}", config_path);
    dbg_log!("[DEBUG] config.json content: {}", json);
    
    // 直接写入目标文件，避免 macOS 上 rename 操作可能因文件系统属性/权限/沙盒问题失败
    std::fs::write(&config_path, &json).map_err(|e| AppError::msg(format!("写入配置文件失败: {}", e)))?;
    
    // 确保数据刷盘
    if let Ok(file) = std::fs::File::open(&config_path) {
        let _ = file.sync_all();
    }

    dbg_log!("[DEBUG] Config saved successfully to: {:?}", config_path);
    Ok(())
}

#[tauri::command]
pub async fn load_settings(app: tauri::AppHandle) -> Result<Settings, AppError> {
    let data_dir = config::get_data_dir(Some(&app))?;
    let config_path = data_dir.join("config.json");

    dbg_log!("[DEBUG] load_settings: reading from {:?}", config_path);
    if !config_path.exists() {
        dbg_log!("[DEBUG] load_settings: config.json not found, returning defaults");
        return Ok(Settings::default());
    }

    let json = std::fs::read_to_string(&config_path).map_err(|e| AppError::msg(format!("读取配置文件失败: {}", e)))?;
    dbg_log!("[DEBUG] load_settings raw json: {}", json);
    let settings: Settings = serde_json::from_str(&json).map_err(|e| AppError::msg(format!("解析配置文件失败: {}", e)))?;
    dbg_log!("[DEBUG] load_settings parsed: {:?}", settings);

    Ok(settings)
}

#[tauri::command]
pub async fn get_app_version(app: tauri::AppHandle) -> Result<String, AppError> {
    let version = app.config().version.clone().unwrap_or_else(|| "0.0.0".to_string());
    Ok(version)
}

#[tauri::command]
pub async fn read_log(date: Option<String>) -> Result<String, AppError> {
    let d = date.unwrap_or_else(|| crate::common::utils::logger::today_str());
    crate::common::utils::logger::read_log(&d)
}

#[tauri::command]
pub async fn list_log_dates() -> Result<Vec<String>, AppError> {
    crate::common::utils::logger::list_log_dates()
}

#[tauri::command]
pub async fn open_log_dir() -> Result<(), AppError> {
    let dir = crate::common::utils::logger::get_log_dir()?;
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(&dir).spawn();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
    }
    Ok(())
}

/// 将当前用户加入 docker 组（Linux 权限修复）。
/// 优先用 pkexec 弹出系统原生 GUI 密码框；pkexec 不可用时回退到终端命令提示。
#[tauri::command]
pub async fn fix_docker_permission() -> Result<String, AppError> {
    #[cfg(target_os = "windows")]
    {
        return Err(AppError::msg("Windows 无需此操作".to_string()));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .map_err(|_| AppError::msg("无法获取当前用户名".to_string()))?;

        // 方案 1：pkexec 弹出 GUI 密码框（GNOME/KDE 桌面默认有 polkit 认证代理）
        let pkexec_result = tokio::process::Command::new("pkexec")
            .args(["usermod", "-aG", "docker", &user])
            .output()
            .await;

        match pkexec_result {
            Ok(output) if output.status.success() => {
                return Ok(format!("已将用户 {} 加入 docker 组，请重启 ADM-BE 后生效", user));
            }
            Ok(output) => {
                // pkexec 执行了但失败（用户取消密码框 / 认证失败）
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                if stderr.contains("Not authorized") || stderr.contains("Request dismissed") || stderr.contains("cancelled") {
                    return Err(AppError::msg("PKEXEC_CANCELLED".to_string()));
                }
                // 其他失败：回退到方案 2
            }
            Err(_) => {
                // pkexec 不存在（headless / 无 polkit），回退到方案 2
            }
        }

        // 方案 2：回退提示——用户在终端手动执行
        Err(AppError::msg(format!(
            "FALLBACK_TERMINAL|{}|请在终端执行以下命令，然后重新登录（注销再登录）后重启 ADM-BE：\n  sudo usermod -aG docker {}\n  sudo systemctl restart docker",
            user, user
        )))
    }
}
