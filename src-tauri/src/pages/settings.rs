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
pub async fn write_app_log(level: String, tag: String, message: String) -> Result<(), AppError> {
    crate::common::utils::logger::write_log(&level, &tag, &message);
    Ok(())
}

// ===== 推理引擎镜像管理 =====

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineImageInfo {
    pub repo_tag: String,
    pub size: String,
    pub created: String,
    pub id: String,
    pub in_use: bool,
}

/// 收集本地已拉取的推理引擎镜像：docker images 过滤仓库名含 sglang 的条目，
/// 并标记当前运行中容器正在使用的镜像（阻止删除）。
fn collect_engine_images(state: &AppState) -> Result<Vec<EngineImageInfo>, AppError> {
    let out = crate::common::utils::platform::docker_cmd()
        .args(["images", "--no-trunc", "--format", "{{.Repository}}\t{{.Tag}}\t{{.ID}}\t{{.Size}}\t{{.CreatedSince}}"])
        .output()
        .map_err(|e| AppError::msg(format!("执行 docker images 失败: {}", e)))?;
    if !out.status.success() {
        return Err(AppError::msg(format!(
            "执行 docker images 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }

    // 当前运行中容器使用的镜像 ID（去掉 sha256: 前缀；--no-trunc 下与 images 输出同为完整 ID）
    let mut use_id: Option<String> = None;
    let container = state.running_container.lock().map_err(|e| e.to_string())?.clone();
    if let Some(c) = container {
        if let Ok(insp) = crate::common::utils::platform::docker_cmd()
            .args(["inspect", "-f", "{{.Image}}", &c])
            .output()
        {
            use_id = String::from_utf8_lossy(&insp.stdout).trim().strip_prefix("sha256:").map(|s| s.trim().to_string());
        }
    }

    let mut images = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 5 {
            continue;
        }
        let repo = parts[0].trim();
        let tag = parts[1].trim();
        if repo.is_empty() || tag.is_empty() || tag == "<none>" {
            continue;
        }
        // 仓库名或 tag 含 sglang 才收（兼容国内镜像源前缀仓库名）
        if !repo.contains("sglang") && !tag.contains("sglang") {
            continue;
        }
        let id = parts[2].trim().to_string();
        images.push(EngineImageInfo {
            repo_tag: format!("{}:{}", repo, tag),
            size: parts[3].trim().to_string(),
            created: parts[4].trim().to_string(),
            in_use: use_id.as_deref() == Some(id.as_str()),
            id,
        });
    }
    images.sort_by(|a, b| b.repo_tag.cmp(&a.repo_tag));
    Ok(images)
}

/// 列出本地已拉取的推理引擎镜像（版本管理面板用）
#[tauri::command]
pub async fn list_engine_images(state: tauri::State<'_, AppState>) -> Result<Vec<EngineImageInfo>, AppError> {
    collect_engine_images(&state)
}

/// 删除指定 tag 的推理引擎镜像；正在被运行中的模型使用时会拒绝删除
#[tauri::command]
pub async fn delete_engine_image(
    state: tauri::State<'_, AppState>,
    repo_tag: String,
) -> Result<(), AppError> {
    for img in collect_engine_images(&state)? {
        if img.repo_tag == repo_tag && img.in_use {
            return Err(AppError::msg("该镜像正在被运行中的模型使用，请先停止模型".to_string()));
        }
    }
    let out = crate::common::utils::platform::docker_cmd()
        .args(["rmi", &repo_tag])
        .output()
        .map_err(|e| AppError::msg(format!("执行 docker rmi 失败: {}", e)))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(AppError::msg(if stderr.is_empty() { "docker rmi 失败".to_string() } else { stderr }));
    }
    Ok(())
}

#[tauri::command]
pub async fn clear_all_logs() -> Result<(), AppError> {
    crate::common::utils::logger::clear_all_logs()
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
                // 权限修复成功，自动注销当前桌面会话（让 docker 组生效）
                // GNOME: gnome-session-quit --logout --no-prompt
                // KDE: qdbus org.kde.ksmserver /KSMServer logout 0 0 0
                // 兜底: loginctl terminate-user $UID
                let _ = tokio::process::Command::new("sh")
                    .args(["-c", "gnome-session-quit --logout --no-prompt 2>/dev/null || qdbus org.kde.ksmserver /KSMServer logout 0 0 0 2>/dev/null || loginctl terminate-user $UID 2>/dev/null"])
                    .spawn();
                return Ok("PERMISSION_FIXED".to_string());
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                if stderr.contains("Not authorized") || stderr.contains("Request dismissed") || stderr.contains("cancelled") {
                    return Err(AppError::msg("PKEXEC_CANCELLED".to_string()));
                }
            }
            Err(_) => {}
        }

        Err(AppError::msg(format!(
            "FALLBACK_TERMINAL|{}|请在终端执行以下命令，然后重新登录（注销再登录）后重启 ADM-BE：\n  sudo usermod -aG docker {}\n  sudo systemctl restart docker",
            user, user
        )))
    }
}
