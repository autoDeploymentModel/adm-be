// settings.html 对应逻辑（配置管理）

use crate::common::*;
use crate::common::config;
use crate::dbg_log;
use crate::app_state::AppState;
use tauri::Emitter;
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

/// 获取当前系统用户名（DGX直连配置本机行 SSH 用户名自动回填）
#[tauri::command]
pub async fn get_local_username() -> Result<String, AppError> {
    for var in ["USER", "LOGNAME", "USERNAME"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return Ok(v);
            }
        }
    }
    Err(AppError::msg("无法获取当前用户名，请在设置中手动填写".to_string()))
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

// ===== 多机互联：节点探活 =====

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub ok: bool,
    /// 远端 GPU 名（如 "GB10"），探不到为空
    pub gpu: String,
    /// 远端 Docker ServerVersion，异常为 "DOCKER_ERR"
    pub docker: String,
    /// 远端是否已下载本机当前使用的 SGLang 镜像
    pub image_ok: bool,
    /// 远端模型目录（含 .done）是否存在
    pub model_exists: bool,
    /// 失败原因（ok=false 时展示给用户）
    pub error: String,
}

/// 探活单个远端节点：SSH 执行 nvidia-smi / docker info / 本机所用镜像 / 模型目录检查。
/// 镜像以本机 config.json 的 sglang_args.image 为准（缺省默认 lmsysorg/sglang:v0.5.17）。
/// 远端无需探测网卡——SSH 可达即说明互联已通。
#[tauri::command]
pub async fn multi_node_probe(
    app: tauri::AppHandle,
    ip: String,
    user: String,
    port: u16,
    key: Option<String>,
    model_dir: Option<String>,
) -> Result<ProbeResult, AppError> {
    crate::common::ssh::validate_host(&ip)?;
    crate::common::ssh::validate_ssh_user(&user)?;
    // 本机当前使用的镜像：设置页配置优先，缺省默认
    let mut image = "lmsysorg/sglang:v0.5.17".to_string();
    if let Ok(settings_path) = config::get_data_dir(Some(&app)).map(|d| d.join("config.json")) {
        if let Ok(json) = std::fs::read_to_string(settings_path) {
            if let Ok(parsed) = serde_json::from_str::<crate::common::types::Settings>(&json) {
                if !parsed.sglang_args.image.trim().is_empty() {
                    image = parsed.sglang_args.image.trim().to_string();
                }
            }
        }
    }

    let cmd = crate::common::ssh::probe_script(model_dir.as_deref().unwrap_or(""), &image);
    let (ok, stdout, stderr) = crate::common::ssh::ssh_run(
        &ip, &user, if port == 0 { 22 } else { port },
        key.as_deref(), &cmd, std::time::Duration::from_secs(20),
    )
    .await?;

    // 解析输出：依次找 GPU:/DOCKER:/IMAGE:/MODEL: 段
    let mut gpu = String::new();
    let mut docker = String::new();
    let mut image_ok = false;
    let mut model_exists = false;
    for line in stdout.lines().chain(stderr.lines()) {
        if let Some(v) = line.strip_prefix("GPU:") {
            gpu = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("DOCKER:") {
            docker = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("IMAGE:") {
            image_ok = v.trim() == "IMAGE_OK";
        } else if let Some(v) = line.strip_prefix("MODEL:") {
            model_exists = v.trim() == "MODEL_OK";
        }
    }

    // SSH 成功但远端环境异常时也返回结构化结果（不直接报错，便于前端逐项展示）
    if !ok {
        return Ok(ProbeResult {
            ok: false,
            gpu,
            docker,
            image_ok,
            model_exists,
            error: if stderr.is_empty() { "命令执行失败".to_string() } else { stderr },
        });
    }
    if docker == "DOCKER_ERR" {
        return Ok(ProbeResult {
            ok: false,
            gpu,
            docker: String::new(),
            image_ok,
            model_exists,
            error: "远端 Docker daemon 不可用或未安装".to_string(),
        });
    }
    if !image_ok {
        return Ok(ProbeResult {
            ok: false,
            gpu,
            docker,
            image_ok,
            model_exists,
            error: format!("远端未下载本机使用的镜像 {}（请先在远端 docker pull 或配置镜像加速）", image),
        });
    }
    Ok(ProbeResult {
        ok: true,
        gpu,
        docker,
        image_ok,
        model_exists,
        error: String::new(),
    })
}

/// 列出本机物理网卡（「互连网卡」下拉/datalist 用）。
/// Linux：/sys/class/net 过滤虚拟接口；Windows：PowerShell Get-NetAdapter（仅开发/探活场景）。
#[tauri::command]
pub async fn list_network_interfaces() -> Result<Vec<String>, AppError> {
    #[cfg(target_os = "windows")]
    {
        let out = tokio::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-NetAdapter -ErrorAction SilentlyContinue | Where-Object {$_.Status -eq 'Up'} | Select-Object -ExpandProperty Name",
            ])
            .output()
            .await
            .map_err(|e| AppError::msg(format!("枚举网卡失败: {}", e)))?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let out = tokio::process::Command::new("sh")
            .args(["-c", "ls /sys/class/net 2>/dev/null"])
            .output()
            .await
            .map_err(|e| AppError::msg(format!("枚举网卡失败: {}", e)))?;
        let mut res = Vec::new();
        for name in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            if name.starts_with("lo")
                || name.starts_with("docker")
                || name.starts_with("veth")
                || name.starts_with("br-")
                || name.starts_with("virbr")
                || name.starts_with("tun")
                || name.starts_with("tap")
            {
                continue;
            }
            res.push(name.to_string());
        }
        Ok(res)
    }
}

// ===== 多机互联：本机网络信息 + SSH 密钥 =====

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetIface {
    pub name: String,
    /// IPv4 地址（空 = 未配置）
    pub ip: String,
    pub is_up: bool,
    /// ConnectX-7 网卡（enp1s0f0np0 / enp1s0f1np1 / enP2p1s0f0np0 / enP2p1s0f1np1）
    pub is_connectx: bool,
    /// RJ45 管理口（enP* 命名，如 enP7s7）
    pub is_manage: bool,
}

/// 扫描本机物理网卡详情：名称 / IPv4 / up-down / ConnectX-7 标记 / 管理口标记。
/// 「多机互联」Tab 主节点配置用：管理网卡自动读取局域网 IP，ConnectX-7 网卡下拉显示状态。
#[tauri::command]
pub async fn get_local_network_info() -> Result<Vec<NetIface>, AppError> {
    #[cfg(target_os = "windows")]
    {
        // Windows 仅开发/探活场景：PowerShell 枚举（无 ConnectX-7 / enP* 命名）
        let out = tokio::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-NetAdapter | ForEach-Object { $ip=(Get-NetIPAddress -InterfaceIndex $_.ifIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue | Select-Object -First 1).IPAddress; \\\"$($_.Name)|$($_.Status)|$ip\\\" }",
            ])
            .output()
            .await
            .map_err(|e| AppError::msg(format!("枚举网卡失败: {}", e)))?;
        let mut res = Vec::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() < 2 {
                continue;
            }
            res.push(NetIface {
                name: parts[0].trim().to_string(),
                ip: parts.get(2).unwrap_or(&"").trim().to_string(),
                is_up: parts.get(1).unwrap_or(&"").trim().eq_ignore_ascii_case("Up"),
                is_connectx: false,
                is_manage: false,
            });
        }
        Ok(res)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let out = tokio::process::Command::new("sh")
            .args([
                "-c",
                "for d in /sys/class/net/*; do n=${d##*/}; case \"$n\" in lo|docker*|veth*|br-*|virbr*|tun*|tap*) continue;; esac; s=$(cat $d/operstate 2>/dev/null); ip=$(ip -o -4 addr show dev $n 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | head -1); echo \"$n|$s|$ip\"; done",
            ])
            .output()
            .await
            .map_err(|e| AppError::msg(format!("扫描网卡失败: {}", e)))?;

        let mut res = Vec::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.is_empty() || parts[0].trim().is_empty() {
                continue;
            }
            let name = parts[0].trim().to_string();
            let is_up = parts.get(1).unwrap_or(&"").trim() == "up";
            let ip = parts.get(2).unwrap_or(&"").trim().to_string();
            // ConnectX-7：BDF 子口命名（s<slot>f<func> + np 后缀）或 enP2p* 别名
            let is_connectx = (name.starts_with("enp") || name.starts_with("enP2p"))
                && (name.ends_with("np0") || name.ends_with("np1") || name.contains("s0f"));
            // RJ45 管理口：enP\n+s\n+ 命名（如 enP7s7）
            let is_manage = name.starts_with("enP")
                && !name.starts_with("enP2p")
                && name[3..].chars().all(|c| c.is_ascii_digit() || c == 'p' || c == 's');
            if is_connectx || is_manage || name.starts_with("en")
                || name.starts_with("eth")
                || name.starts_with("wl")
            {
                res.push(NetIface {
                    name,
                    ip,
                    is_up,
                    is_connectx,
                    is_manage,
                });
            }
        }
        Ok(res)
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyInfo {
    pub key_path: String,
    pub public_key: String,
    /// 本次是否新建（false = 检测到已存在直接复用）
    pub created: bool,
}

/// 确保 ~/.ssh/id_ed25519 存在（已存在则复用不重复生成），返回公钥供加入远端 authorized_keys。
#[tauri::command]
pub async fn ensure_ssh_key() -> Result<SshKeyInfo, AppError> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| AppError::msg("无法获取用户主目录".to_string()))?;
    let ssh_dir = std::path::Path::new(&home).join(".ssh");
    let key_path = ssh_dir.join("id_ed25519");
    let pub_path = ssh_dir.join("id_ed25519.pub");

    if key_path.exists() && pub_path.exists() {
        let pk = std::fs::read_to_string(&pub_path)
            .map_err(|e| AppError::msg(format!("读取公钥失败: {}", e)))?;
        return Ok(SshKeyInfo {
            key_path: key_path.to_string_lossy().to_string(),
            public_key: pk.trim().to_string(),
            created: false,
        });
    }

    std::fs::create_dir_all(&ssh_dir)
        .map_err(|e| AppError::msg(format!("创建 ~/.ssh 失败: {}", e)))?;
    let out = tokio::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", ""])
        .arg("-f")
        .arg(&key_path)
        .args(["-C", "adm-multinode"])
        .output()
        .await
        .map_err(|e| AppError::msg(format!("执行 ssh-keygen 失败（本机未安装 OpenSSH？）: {}", e)))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(AppError::msg(if stderr.is_empty() { "生成 SSH Key 失败".to_string() } else { stderr }));
    }
    let pk = std::fs::read_to_string(&pub_path)
        .map_err(|e| AppError::msg(format!("读取公钥失败: {}", e)))?;
    Ok(SshKeyInfo {
        key_path: key_path.to_string_lossy().to_string(),
        public_key: pk.trim().to_string(),
        created: true,
    })
}

/// 返回软件数据目录（config.json / models 所在位置；「DGX直连配置」本机模型目录自动填引用）
#[tauri::command]
pub async fn get_app_data_dir(app: tauri::AppHandle) -> Result<String, AppError> {
    Ok(config::get_data_dir(Some(&app))?.to_string_lossy().to_string())
}

// ===== 多机互联：镜像/模型一键同步（流式管道，不落盘）=====

fn pipe_emit(app: &tauri::AppHandle, ip: &str, event: &str, phase: &str, percent: u8, detail: &str) {
    let _ = app.emit(event, serde_json::json!({
        "ip": ip, "phase": phase, "percent": percent, "detail": detail,
    }));
}

/// 从 stderr 字节流提取最近一个 "NN%"（pv / rsync progress2 进度），无则 None
fn extract_percent(s: &str) -> Option<u8> {
    let bytes = s.as_bytes();
    let mut last: Option<u8> = None;
    for i in 0..bytes.len() {
        if bytes[i] == b'%' {
            let mut num: u16 = 0;
            let mut k: u32 = 0;
            let mut j = i;
            while j > 0 && k < 3 && bytes[j - 1].is_ascii_digit() {
                num += (bytes[j - 1] - b'0') as u16 * 10u16.pow(k);
                k += 1;
                j -= 1;
            }
            if k > 0 {
                last = Some(num.min(100) as u8);
            }
        }
    }
    last
}

/// 执行流式管道（sh -c）：stderr 解析百分比实时发进度事件，返回 (退出成功, stdout, stderr尾段)。
/// 超时在内部处理并强制 kill 子进程（防止 timeout 包裹 future drop 时 sh 进程泄漏）。
async fn run_pipe_with_progress(
    app: &tauri::AppHandle,
    ip: &str,
    event: &str,
    script: &str,
    timeout: std::time::Duration,
) -> Result<(bool, String, String), AppError> {
    let mut child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| AppError::msg(format!("启动管道命令失败: {}", e)))?;

    let result = tokio::time::timeout(timeout, async {
        // stderr：流式解析百分比（pv 的 " 12%" / rsync progress2 的 " 12%"），并保留尾部错误信息
        let mut err_tail = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let mut buf = vec![0u8; 8192];
            let mut last_pct: u8 = 0;
            loop {
                let n = tokio::io::AsyncReadExt::read(&mut stderr, &mut buf).await.map_err(|e| AppError::msg(format!("读取管道输出失败: {}", e)))?;
                if n == 0 {
                    break;
                }
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some(pct) = extract_percent(&s) {
                    if pct != last_pct && pct >= 1 && pct <= 99 {
                        last_pct = pct;
                        pipe_emit(app, ip, event, "transfer", pct, "");
                    }
                }
                // 保留尾部 4KB 供失败诊断
                err_tail.push_str(&s);
                if err_tail.len() > 4096 {
                    err_tail = err_tail[err_tail.len() - 4096..].to_string();
                }
            }
        }

        let mut stdout = String::new();
        if let Some(mut so) = child.stdout.take() {
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut so, &mut stdout).await;
        }
        let status = child.wait().await.map_err(|e| AppError::msg(format!("等待管道命令失败: {}", e)))?;
        let result_ok = status.success();
        if !result_ok && !stdout.trim().is_empty() {
            // 退出失败时 stdout（含 docker load 报错）随错误返回
            return Err(AppError::msg(stdout.trim().to_string()));
        }
        Ok::<(bool, String, String), AppError>((result_ok, stdout, err_tail))
    })
    .await;

    match result {
        Ok(r) => r,
        Err(_) => {
            let _ = child.kill();
            Err(AppError::msg(format!("管道执行超时（{}s）", timeout.as_secs())))
        }
    }
}

fn ssh_opts(port: u16, key: Option<&str>) -> String {
    let mut s = format!("-o BatchMode=yes -o ConnectTimeout=5 -o StrictHostKeyChecking=accept-new -p {}", port);
    if let Some(k) = key {
        s.push_str(&format!(" -i {}", k));
    }
    s
}

/// 一键流式同步本机 SGLang 镜像到远端：
/// `docker save <image> | [pv -s <size>] | gzip -1 | ssh <直连IP> 'gunzip | docker load'`
/// 全程不落地临时 tar；进度经 `image-push-progress` 事件实时上报。
#[tauri::command]
pub async fn push_image_to_remote(
    app: tauri::AppHandle,
    ip: String,
    user: String,
    port: u16,
    key: Option<String>,
    image: String,
) -> Result<String, AppError> {
    crate::common::ssh::validate_host(&ip)?;
    crate::common::ssh::validate_ssh_user(&user)?;
    let image = image.trim();
    if image.is_empty() {
        return Err(AppError::msg("镜像名为空".to_string()));
    }
    let ssh_port = if port == 0 { 22 } else { port };
    let key_ref = key.as_deref().filter(|k| !k.trim().is_empty()).map(str::trim);

    // 镜像总大小（供 pv -s 显示总进度）；pv 不存在时跳过
    let mut size = String::new();
    if let Ok(out) = tokio::process::Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Size}}", &image])
        .output()
        .await
    {
        if out.status.success() {
            size = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
    }
    let has_pv = tokio::process::Command::new("sh")
        .args(["-c", "command -v pv >/dev/null 2>&1 && echo 1 || echo 0"])
        .output()
        .await
        .map(|o| o.stdout.starts_with(b"1"))
        .unwrap_or(false);

    pipe_emit(&app, &ip, "image-push-progress", "save", 3, "导出并流式传输中...");
    let remote_cmd = format!(
        "gunzip | docker load && docker image inspect {} >/dev/null 2>&1 && echo LOAD_OK || echo LOAD_FAIL",
        crate::common::ssh::sh_quote(&image)
    );
    let pipe = if has_pv && !size.is_empty() {
        format!(
            "docker save {} | pv -f -s {} | gzip -1 | ssh {} {}@{} \"{}\"",
            crate::common::ssh::sh_quote(&image),
            size,
            ssh_opts(ssh_port, key_ref),
            user.trim(),
            ip.trim(),
            remote_cmd
        )
    } else {
        format!(
            "docker save {} | gzip -1 | ssh {} {}@{} \"{}\"",
            crate::common::ssh::sh_quote(&image),
            ssh_opts(ssh_port, key_ref),
            user.trim(),
            ip.trim(),
            remote_cmd
        )
    };

    let (ok, stdout, err_tail) = run_pipe_with_progress(&app, &ip, "image-push-progress", &pipe, std::time::Duration::from_secs(600)).await
        .map_err(|e| {
            if e.to_string().contains("超时") {
                AppError::msg(format!("{}\n（镜像同步超时，请检查光口连通）", e))
            } else {
                e
            }
        })?;
    if !ok || !stdout.contains("LOAD_OK") {
        if !err_tail.trim().is_empty() {
            return Err(AppError::msg(format!("远端镜像导入失败：\n{}\n{}", stdout.trim(), err_tail.trim())));
        }
        return Err(AppError::msg(format!("远端镜像导入失败：\n{}", stdout.trim())));
    }
    pipe_emit(&app, &ip, "image-push-progress", "done", 100, "镜像同步完成");
    Ok(format!("镜像 {} 已流式同步到 {}", image, ip))
}

/// 一键同步本机所有已下载模型到直连节点（rsync 增量优先，无 rsync 回退 scp -r；
/// 进度经 `model-sync-progress` 事件上报；完成校验远端 .done）。
/// 全量同步：遍历本机全部完整模型，正在运行的模型排最前；
/// 远端目标 = 设置页模型目录（留空自动 /home/<SSH用户>/models/<模型ID>），
/// 已同步（远端 .done 存在）的模型自动跳过。
#[tauri::command]
pub async fn sync_model_to_remote(
    app: tauri::AppHandle,
    ip: String,
    user: String,
    port: u16,
    key: Option<String>,
    remote_model_dir: String,
) -> Result<String, AppError> {
    crate::common::ssh::validate_host(&ip)?;
    crate::common::ssh::validate_ssh_user(&user)?;
    let data_dir = config::get_data_dir(Some(&app))?;
    let models_root = data_dir.join("models");

    // 本机完整模型列表（.done 或 config.json + model.safetensors）
    let mut local_models: Vec<(String, std::path::PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&models_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let complete = path.join(".done").exists()
                || (path.join("config.json").exists() && path.join("model.safetensors").exists());
            if complete {
                if let Some(name) = path.file_name() {
                    local_models.push((name.to_string_lossy().to_string(), path));
                }
            }
        }
    }
    if local_models.is_empty() {
        return Err(AppError::msg("本机没有已下载完成的模型，请先下载模型".to_string()));
    }
    // 正在运行的模型排最前（优先同步），其余保持顺序
    if let Ok(guard) = app.state::<AppState>().running_model_id.lock() {
        if let Some(rid) = guard.as_ref() {
            if let Some(pos) = local_models.iter().position(|(id, _)| id == rid) {
                let m = local_models.remove(pos);
                local_models.insert(0, m);
            }
        }
    }

    let ssh_port = if port == 0 { 22 } else { port };
    let key_ref = key.as_deref().filter(|k| !k.trim().is_empty()).map(str::trim);
    let opts = ssh_opts(ssh_port, key_ref);

    // 远端模型根目录：留空自动 /home/<user>/models（user 空时 ~/models），填写则按填写值
    let base = remote_model_dir.trim();
    let base = if base.is_empty() {
        let u = user.trim();
        if u.is_empty() {
            "~/models".to_string()
        } else {
            format!("/home/{}/models", u)
        }
    } else {
        base.to_string()
    };

    let mut synced = 0usize;
    let mut skipped = 0usize;
    for (model_id, src) in &local_models {
        let remote_dir = format!("{}/{}", base, model_id);
        let chk_q = crate::common::ssh::quote_remote_path(&remote_dir);
        // 监测：远端是否已同步（.done 是文件，用 -e 而非 -d）
        let chk = format!("test -e {}/.done && echo MODEL_OK || echo MODEL_MISSING", chk_q);
        let (cok, cout, _) = crate::common::ssh::ssh_run(&ip, &user, ssh_port, key_ref, &chk, std::time::Duration::from_secs(20)).await?;
        if cok && cout.contains("MODEL_OK") {
            skipped += 1;
            continue;
        }

        pipe_emit(&app, &ip, "model-sync-progress", "save", 0, &format!("同步模型 {} ...", model_id));
        let src_str = format!("{}/", src.to_string_lossy());
        // 先远端建目录（rsync 不会自动创建不存在的多级父目录），再增量 rsync；无 rsync 回退 scp -r
        let remote_dir_q = crate::common::ssh::quote_remote_path(&remote_dir);
        let script = format!(
            "ssh {} '{}@{}' 'mkdir -p {}' 2>&1 && \
             if command -v rsync >/dev/null 2>&1; then \
               rsync -a --info=progress2 --no-inc-recursive -e 'ssh {}' '{}' '{}@{}:{}' 2>&1; \
             else \
               scp -r -o BatchMode=yes -o ConnectTimeout=5 '{}' '{}@{}:{}' 2>&1; \
             fi",
            opts,
            user.trim(),
            ip.trim(),
            remote_dir_q,
            opts,
            src_str,
            user.trim(),
            ip.trim(),
            crate::common::ssh::quote_remote_path(&format!("{}/", remote_dir)),
            src_str,
            user.trim(),
            ip.trim(),
            crate::common::ssh::quote_remote_path(&format!("{}/", remote_dir)),
        );

        let (ok, _, err_tail) = run_pipe_with_progress(&app, &ip, "model-sync-progress", &script, std::time::Duration::from_secs(3600)).await
            .map_err(|e| {
                if e.to_string().contains("超时") {
                    AppError::msg(format!("{}\n（模型同步超时，请检查光口连通与磁盘空间）", e))
                } else {
                    e
                }
            })?;
        if !ok {
            let detail = if err_tail.trim().is_empty() {
                "请检查直连节点磁盘空间与目录权限".to_string()
            } else {
                err_tail.trim().to_string()
            };
            return Err(AppError::msg(format!("同步模型 {} 失败：\n{}", model_id, detail)));
        }

        // 校验远端 .done
        let (cok2, cout2, _) = crate::common::ssh::ssh_run(&ip, &user, ssh_port, key_ref, &chk, std::time::Duration::from_secs(20)).await?;
        if !cok2 || !cout2.contains("MODEL_OK") {
            return Err(AppError::msg(format!(
                "模型 {} 已传输但远端校验失败（.done 缺失），请确认远端模型目录路径正确",
                model_id
            )));
        }
        synced += 1;
    }
    pipe_emit(&app, &ip, "model-sync-progress", "done", 100, "模型同步完成");
    if synced == 0 {
        Ok(format!("全部模型已同步，无需重复（跳过 {} 个已存在）", skipped))
    } else {
        Ok(format!("模型同步完成：新同步 {} 个，跳过 {} 个已同步", synced, skipped))
    }
}
