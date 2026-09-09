// index.html 对应逻辑（硬件信息、全局更新）

use crate::common::*;
use crate::app_state::AppState;
use crate::common::utils::platform;
use crate::bail;

// ===== 辅助函数 =====

#[allow(dead_code)]
fn compare_versions(current: &str, remote: &str) -> std::cmp::Ordering {
    let parse_version = |v: &str| -> Vec<u32> {
        v.trim()
            .split('.')
            .filter_map(|s| s.parse::<u32>().ok())
            .collect()
    };

    let cur_parts = parse_version(current);
    let rem_parts = parse_version(remote);

    for i in 0..cur_parts.len().max(rem_parts.len()) {
        let cur = cur_parts.get(i).copied().unwrap_or(0);
        let rem = rem_parts.get(i).copied().unwrap_or(0);
        if cur < rem {
            return std::cmp::Ordering::Less;
        }
        if cur > rem {
            return std::cmp::Ordering::Greater;
        }
    }
    std::cmp::Ordering::Equal
}

// ===== Tauri Command =====

/// 拉取远程更新清单 update.json（15s 超时）。
pub(crate) async fn fetch_update_info() -> Result<UpdateInfo, AppError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let response = client
        .get("https://adm.tuduoduo.top/b/update.json")
        .send()
        .await
        .map_err(|e| format!("检查更新失败: {}", e))?;

    if !response.status().is_success() {
        bail!("服务器返回错误状态码: {}", response.status());
    }

    let text = response
        .text()
        .await
        .map_err(|e| format!("读取响应文本失败: {}", e))?;

    serde_json::from_str(&text).map_err(|e| AppError::msg(format!("解析更新信息失败: {}", e)))
}

#[tauri::command]
pub async fn get_system_info(state: tauri::State<'_, AppState>) -> Result<SystemInfo, AppError> {
    let mut sys = state.sys.lock().map_err(|e| format!("锁获取失败: {}", e))?;
    sys.refresh_all();

    let total_ram = sys.total_memory();
    let used_ram = sys.used_memory();
    let cpu_usage = sys.global_cpu_usage();
    let cpu_physical_cores = sys.physical_core_count().unwrap_or(0);
    let cpu_logical_cores = sys.cpus().len();

    let (total_vram, used_vram, has_gpu) = platform::get_gpu_info();

    Ok(SystemInfo {
        total_ram,
        used_ram,
        total_vram,
        used_vram,
        has_gpu,
        cpu_usage,
        cpu_physical_cores,
        cpu_logical_cores,
    })
}

#[tauri::command]
pub async fn check_update(app: tauri::AppHandle) -> Result<UpdateCheckResult, AppError> {
    let current_version = app.config().version.clone().unwrap_or_else(|| "0.0.0".to_string());

    let update_info = fetch_update_info().await?;

    // 目前仅提供 linux-arm64（.deb 手动分发）更新通道；
    // 其他平台（Windows / macOS / Linux x64）一律不弹"发现新版本"，
    // 避免前端 openUrl('null') 打开无效下载链接。
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    let has_update =
        compare_versions(&current_version, &update_info.version) == std::cmp::Ordering::Less;

    #[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
    let has_update = false;

    let download_url;
    let changelog_url;

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        download_url = update_info.linux_arm64.as_ref().map(|p| p.app_url.clone());
        changelog_url = None;
    }

    #[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
    {
        download_url = None;
        changelog_url = None;
    }

    Ok(UpdateCheckResult {
        has_update,
        remote_version: update_info.version,
        current_version,
        download_url,
        changelog_url,
    })
}
