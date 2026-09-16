//! ComfyUI 服务（「视频生成」页）配套后端：
//! - `comfyui_setup_status`：检查本地镜像是否已就绪、权重是否已下载（供页面显示状态）
//! - `pull_comfyui_image`：`docker pull` 拉取镜像（模型清单 `engine_image` 必填）
//!
//! 说明：ComfyUI 官方无 ARM64 镜像，镜像由手工脚本 `scripts/docker/h3-comfyui/`
//! （Dockerfile + build-image.sh）在离线/其它机器上构建并推送到 registry；应用内
//! 不再提供构建流程，只负责拉取与启动（启动路径缺镜像时同样按 registry 拉取）。

use crate::bail;
use crate::common::config;
use crate::common::error::AppError;
use crate::common::utils::platform;

/// 页面上展示的安装状态
#[derive(serde::Serialize)]
pub struct ComfyuiSetupStatus {
    /// 目标镜像名（= 模型清单 engine_image）
    pub image: String,
    /// 本地镜像是否已存在
    pub image_exists: bool,
    /// 权重目录（<data>/models/<model_id>）
    pub weights_dir: String,
    /// 权重是否就绪（diffusion_models/ 与 text_encoders/ 下各有 ≥1 个 .safetensors）
    pub weights_downloaded: bool,
    /// 是否存在断点续传现场（目录里有 `.aria2` 控件）→ 页面按钮显示「继续下载」
    pub weights_partial: bool,
    /// 权重目录下 .safetensors 总字节数（页面折算 GB 展示）
    pub weights_bytes: u64,
}

/// 判断 docker 镜像是否存在于本机（`docker image inspect` 退出码）。
async fn image_exists_locally(image: &str) -> bool {
    let out = platform::docker_cmd_tokio()
        .args(["image", "inspect", image])
        .output();
    match tokio::time::timeout(std::time::Duration::from_secs(20), out).await {
        Ok(Ok(o)) => o.status.success(),
        _ => false,
    }
}

/// 统计目录下（含一层子目录）指定类型文件的数量与已落盘字节：
/// 数量只计已完成文件（有 `.aria2` 续传控件或未到齐的不算）；字节含续传中文件的已落盘部分，
/// 并报告目录树里是否存在续传控件（`partial=true` 即仍在下载中）。
fn scan_dir_counts(dir: &std::path::Path, depth: usize) -> (usize, u64, bool) {
    let mut count = 0usize;
    let mut bytes = 0u64;
    let mut partial = false;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0, false);
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 1 {
                let (c, b, pf) = scan_dir_counts(&p, depth - 1);
                count += c;
                bytes += b;
                partial |= pf;
            }
            continue;
        }
        // 续传控件：有它说明同目录下仍有未完成文件（即使长度已到齐）
        if p.extension().map(|x| x.eq_ignore_ascii_case("aria2")).unwrap_or(false) {
            partial = true;
            continue;
        }
        let is_model = p
            .extension()
            .map(|x| x.eq_ignore_ascii_case("safetensors") || x.eq_ignore_ascii_case("gguf"))
            .unwrap_or(false);
        if !is_model {
            continue;
        }
        // aria2 预分配会让未完成文件也达到全量长度：有同名 .aria2 控件时不计入完成数，
        // 但已落盘字节仍计入（页面「已中断（已下载 xx GB）」不能只算完成文件）
        let aria2 = p.with_file_name(format!(
            "{}.aria2",
            p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        ));
        if let Ok(m) = e.metadata() {
            if aria2.exists() {
                partial = true;
                bytes += crate::common::utils::download::file_allocated_bytes(&m);
            } else {
                count += 1;
                bytes += m.len();
            }
        }
    }
    (count, bytes, partial)
}

/// ComfyUI 安装状态：镜像是否已就绪 + 权重是否已下载。
#[tauri::command]
pub async fn comfyui_setup_status(
    app: tauri::AppHandle,
    model_id: String,
    image: String,
) -> Result<ComfyuiSetupStatus, AppError> {
    let data_dir = config::get_data_dir(Some(&app))?;
    let weights_dir = data_dir.join("models").join(&model_id);

    let image = image.trim().to_string();
    let image_exists = if image.is_empty() {
        false
    } else {
        image_exists_locally(&image).await
    };

    let (dit_count, dit_bytes, dit_partial) = scan_dir_counts(&weights_dir.join("diffusion_models"), 2);
    let (enc_count, enc_bytes, enc_partial) = scan_dir_counts(&weights_dir.join("text_encoders"), 2);

    Ok(ComfyuiSetupStatus {
        image,
        image_exists,
        weights_dir: weights_dir.to_string_lossy().to_string(),
        // 仍在断点续传（存在 .aria2 控件）时不算已下载，页面继续提供「下载权重」按钮
        weights_downloaded: dit_count > 0 && enc_count > 0 && !dit_partial && !enc_partial,
        weights_partial: dit_partial || enc_partial,
        weights_bytes: dit_bytes + enc_bytes,
    })
}

/// 下载（拉取）ComfyUI 镜像：`engine_image` 形如 `<registry>/<ns>/<name>:<tag>`
/// （手工脚本构建后推送到 registry 的产物）。复用模型下载链路的拉取实现
/// （`docker_preflight` 预检 / 镜像已存在即跳过 / 空闲超时后自动重试一次 /
/// `model-pull-progress` 进度事件 / `model-log` 输出），成功后才算「环境准备」的镜像就绪。
#[tauri::command]
pub async fn pull_comfyui_image(
    app: tauri::AppHandle,
    model_id: String,
    image: String,
) -> Result<(), AppError> {
    let image = image.trim().to_string();
    if image.is_empty() {
        bail!("缺少镜像名（模型清单 engine_image 必填）");
    }
    crate::pages::model_list::pull_docker_image(&app, &model_id, &image).await?;
    Ok(())
}
