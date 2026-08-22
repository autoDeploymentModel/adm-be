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
    pub id: String,
    pub in_use: bool,
}

/// 收集本地已拉取的推理引擎镜像：docker images 过滤仓库名含 sglang 的条目，
/// 并标记当前运行中容器正在使用的镜像（阻止删除）。
fn collect_engine_images(state: &AppState) -> Result<Vec<EngineImageInfo>, AppError> {
    // 用 JSON 格式输出解析（tab 列解析在部分 docker 版本上字段会错位/缺失）
    let out = crate::common::utils::platform::docker_cmd()
        .args(["images", "--no-trunc", "--format", "{{json .}}"])
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
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let repo = v["Repository"].as_str().unwrap_or("").trim();
        let tag = v["Tag"].as_str().unwrap_or("").trim();
        if repo.is_empty() || tag.is_empty() || tag == "<none>" {
            continue;
        }
        // 仓库名或 tag 含 sglang 才收（兼容国内镜像源前缀仓库名）
        if !repo.contains("sglang") && !tag.contains("sglang") {
            continue;
        }
        let id = v["ID"].as_str().unwrap_or("").trim().to_string();
        images.push(EngineImageInfo {
            repo_tag: format!("{}:{}", repo, tag),
            size: v["Size"].as_str().unwrap_or("-").trim().to_string(),
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

// ===== Docker 镜像加速配置（daemon.json registry-mirrors）=====

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerMirrorConfig {
    pub daemon_path: String,
    pub exists: bool,
    pub mirrors: Vec<String>,
    pub platform: &'static str,
}

/// 各平台 Docker daemon.json 路径（Linux/DGX Spark: /etc/docker/daemon.json；
/// Windows Docker Desktop: C:\ProgramData\Docker\config\daemon.json）
fn docker_daemon_path() -> String {
    if cfg!(target_os = "windows") {
        "C:\\ProgramData\\Docker\\config\\daemon.json".to_string()
    } else {
        "/etc/docker/daemon.json".to_string()
    }
}

/// 读取 Docker daemon.json 的 registry-mirrors 配置（用于镜像加速，仅影响后续拉取）
#[tauri::command]
pub async fn get_docker_mirror_config() -> Result<DockerMirrorConfig, AppError> {
    let path = docker_daemon_path();
    let path_obj = std::path::Path::new(&path);
    let exists = path_obj.exists();
    let mirrors = if exists {
        std::fs::read_to_string(path_obj)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| {
                v["registry-mirrors"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(DockerMirrorConfig {
        daemon_path: path,
        exists,
        mirrors,
        platform: if cfg!(target_os = "windows") { "windows" } else { "linux" },
    })
}

/// 保存 registry-mirrors 到 Docker daemon.json 并重启 Docker 服务使配置生效。
/// Linux：pkexec 提权写 /etc/docker/daemon.json + systemctl/service 重启；
/// Windows：UAC 提权写 ProgramData + 重启 Docker Desktop。
#[tauri::command]
pub async fn save_docker_mirror_config(mirrors: Vec<String>) -> Result<String, AppError> {
    // 1. 清洗输入：去空白、去重复
    let mut cleaned: Vec<String> = Vec::new();
    for m in mirrors {
        let t = m.trim();
        if t.is_empty() || cleaned.contains(&t.to_string()) {
            continue;
        }
        cleaned.push(t.to_string());
    }

    // 2. 读取现有 daemon.json（不存在则空对象），仅更新 registry-mirrors，保留其他字段
    let path = docker_daemon_path();
    let path_obj = std::path::Path::new(&path);
    let mut daemon: serde_json::Value = if path_obj.exists() {
        std::fs::read_to_string(path_obj)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    daemon["registry-mirrors"] = serde_json::Value::Array(
        cleaned.iter().map(|m| serde_json::Value::String(m.clone())).collect(),
    );

    // 3. 写入临时文件（当前用户可写），再提权安装到 daemon.json
    let tmp_dir = std::env::temp_dir();
    let tmp_json = tmp_dir.join("adm-daemon.json");
    std::fs::write(&tmp_json, serde_json::to_string_pretty(&daemon).map_err(|e| AppError::msg(format!("序列化 daemon.json 失败: {}", e)))?)
        .map_err(|e| AppError::msg(format!("写入临时文件失败: {}", e)))?;

    #[cfg(target_os = "windows")]
    {
        // UAC 提权：copy 临时文件到 ProgramData，重启 Docker Desktop
        let ps1 = tmp_dir.join("adm-docker-mirror.ps1");
        let script = format!(
            "Copy-Item -Force '{}' '{}'\n\
             Restart-Service com.docker.service -Force -ErrorAction SilentlyContinue\n\
             Get-Process 'Docker Desktop' -ErrorAction SilentlyContinue | Stop-Process -Force\n\
             Start-Sleep -Seconds 2\n\
             Start-Process 'C:\\Program Files\\Docker\\Docker\\Docker Desktop.exe' -ErrorAction SilentlyContinue\n",
            tmp_json.display(),
            path
        );
        std::fs::write(&ps1, script).map_err(|e| AppError::msg(format!("写入提权脚本失败: {}", e)))?;
        let out = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-Command"])
            .arg(format!(
                "Start-Process powershell -Verb RunAs -Wait -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','{}'",
                ps1.display()
            ))
            .output()
            .await
            .map_err(|e| AppError::msg(format!("启动提权进程失败: {}", e)))?;
        if out.status.success() {
            return Ok("DOCKER_RESTARTED".to_string());
        }
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        // 兼容英美拼写：canceled（美）/ cancelled（英）
        if stderr.contains("cancel") || stderr.contains("denied") {
            return Err(AppError::msg("UAC_CANCELLED".to_string()));
        }
        return Err(AppError::msg(format!(
            "FALLBACK_MANUAL|请在管理员 PowerShell 中执行：\n  Copy-Item -Force '{}' '{}'\n  然后重启 Docker Desktop",
            tmp_json.display(),
            path
        )));
    }

    #[cfg(not(target_os = "windows"))]
    {
        // pkexec 提权：安装 daemon.json + 重启 Docker（systemd 优先，回退 service）
        let cmd = format!(
            "install -m 0644 '{}' '{}' && (systemctl restart docker 2>/dev/null || service docker restart 2>/dev/null)",
            tmp_json.display(),
            path
        );
        let out = tokio::process::Command::new("pkexec")
            .args(["sh", "-c", &cmd])
            .output()
            .await
            .map_err(|e| AppError::msg(format!("启动 pkexec 失败: {}", e)))?;
        if out.status.success() {
            return Ok("DOCKER_RESTARTED".to_string());
        }
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if stderr.contains("Not authorized") || stderr.contains("dismissed") || stderr.contains("cancel") {
            return Err(AppError::msg("PKEXEC_CANCELLED".to_string()));
        }
        return Err(AppError::msg(format!(
            "FALLBACK_MANUAL|请在终端执行以下命令并重启 Docker 服务：\n  sudo install -m 0644 '{}' '{}'\n  sudo systemctl restart docker",
            tmp_json.display(),
            path
        )));
    }
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
