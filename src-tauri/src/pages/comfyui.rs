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
    /// 权重是否就绪（下载清单里的文件全部到齐）
    pub weights_downloaded: bool,
    /// 是否存在断点续传现场（文件已部分落盘）→ 页面按钮显示「继续下载」
    pub weights_partial: bool,
    /// 已落盘字节（已完成文件按预期大小计；进行中文件按已落盘部分计）
    pub weights_bytes: u64,
    /// 下载清单里的期望总字节（0 = 清单不可解析 / 大小未知）
    pub weights_total_bytes: u64,
    /// 未到齐的文件（相对权重目录，最多 8 条，供页面 tooltip 显示）
    pub weights_missing: Vec<String>,
    /// 未到齐文件总数
    pub weights_missing_count: usize,
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
        let aria2 = sidecar_path(&p).exists();
        if let Ok(m) = e.metadata() {
            if aria2 {
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

/// 权重完整性判定结果
struct WeightsReport {
    downloaded: bool,
    partial: bool,
    bytes: u64,
    total: u64,
    missing: Vec<String>,
    missing_count: usize,
}

/// 续传控件路径（`<file>.aria2`）
fn sidecar_path(path: &std::path::Path) -> std::path::PathBuf {
    path.with_file_name(format!(
        "{}.aria2",
        path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    ))
}

/// 解析下载清单 → 期望文件列表（相对权重目录的路径 + 预期字节数；0 = 大小未知）。
/// 仓库条目解密经由 HF API 文件清单按 include 过滤（与 hfd 下载同一套语义），
/// API 不可达时退回 hfd 自身的 `.hfd/manifest`；纯 URL 条目只按文件名判定存在性。
/// 清单为空/无法解析时返回 None（调用方退回旧启发式判定）。
async fn resolve_expected_files(
    app: &tauri::AppHandle,
    weights_dir: &std::path::Path,
    model_files: &[String],
) -> Option<Vec<(String, u64)>> {
    if model_files.is_empty() {
        return None;
    }
    if !model_files.iter().any(|f| crate::pages::model_list::is_repo_entry(f)) {
        let out: Vec<(String, u64)> = model_files
            .iter()
            .filter_map(|f| f.rsplit('/').next())
            .filter(|n| !n.is_empty())
            .map(|n| (n.to_string(), 0u64))
            .collect();
        return if out.is_empty() { None } else { Some(out) };
    }

    // 端点策略与下载链路一致：未配代理 → hf-mirror；配代理 → huggingface.co
    let proxy = crate::common::utils::proxy::proxy_url(app).await;
    let endpoint = if proxy.is_empty() { "https://hf-mirror.com" } else { "https://huggingface.co" };
    let http = crate::common::utils::proxy::build_download_http(
        app,
        Some(std::time::Duration::from_secs(10)),
    )
    .await
    .ok()?;

    let mut out: Vec<(String, u64)> = Vec::new();
    for (repo, includes) in crate::pages::model_list::merge_repo_entries(model_files) {
        let files = match crate::pages::model_list::fetch_repo_files(&http.client, endpoint, &repo).await {
            Some(files) => files,
            // API 不可达（离线/弱网）：退回 hfd 清单（内容已按 --include 过滤）
            None => crate::pages::model_list::read_hfd_manifest(weights_dir)?,
        };
        out.extend(
            files
                .into_iter()
                .filter(|(name, _)| crate::pages::model_list::matches_include_patterns(name, &includes)),
        );
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 按期望清单判定权重完整性：
/// 已完成 = 文件在且达到预期大小——无 `.aria2` 控件时看文件长度；
/// 有控件时看**已分配块数**（aria2 按段稀疏写盘，未写入的段不占块，文件长度会提前逼近全量），
/// 因此「后来由 ComfyUI 弹窗/手工补齐、只剩陈旧 `.aria2` 控件」的文件不再被误判成未完成。
fn inspect_expected_weights(
    weights_dir: &std::path::Path,
    expected: &[(String, u64)],
) -> WeightsReport {
    use crate::common::utils::download::file_allocated_bytes;
    let mut r = WeightsReport {
        downloaded: false,
        partial: false,
        bytes: 0,
        total: 0,
        missing: Vec::new(),
        missing_count: 0,
    };
    for (rel, size) in expected {
        r.total += *size;
        let path = weights_dir.join(rel);
        let meta = std::fs::metadata(&path).ok();
        let sidecar = sidecar_path(&path).exists();
        if meta.is_some() || sidecar {
            r.partial = true;
        }
        let done = match &meta {
            None => false,
            Some(m) => {
                if *size == 0 {
                    // 大小未知（URL 条目）：无续传控件即视为完成
                    !sidecar
                } else if sidecar {
                    file_allocated_bytes(m) >= *size
                } else {
                    m.len() >= *size
                }
            }
        };
        if done {
            r.bytes += if *size > 0 {
                *size
            } else {
                meta.as_ref().map(|m| m.len()).unwrap_or(0)
            };
            continue;
        }
        r.bytes += match &meta {
            None => 0,
            Some(m) if *size > 0 => file_allocated_bytes(m).min(*size),
            Some(m) => file_allocated_bytes(m),
        };
        r.missing_count += 1;
        if r.missing.len() < 8 {
            r.missing.push(rel.clone());
        }
    }
    r.downloaded = r.missing_count == 0;
    if r.downloaded {
        r.partial = false;
    }
    r
}

/// 无法解析清单时的旧启发式：diffusion_models/ 与 text_encoders/ 各 ≥1 个 .safetensors 且无续传控件。
fn inspect_dir_heuristic(weights_dir: &std::path::Path) -> WeightsReport {
    let (dit_count, dit_bytes, dit_partial) = scan_dir_counts(&weights_dir.join("diffusion_models"), 2);
    let (enc_count, enc_bytes, enc_partial) = scan_dir_counts(&weights_dir.join("text_encoders"), 2);
    WeightsReport {
        downloaded: dit_count > 0 && enc_count > 0 && !dit_partial && !enc_partial,
        partial: dit_partial || enc_partial,
        bytes: dit_bytes + enc_bytes,
        total: 0,
        missing: Vec::new(),
        missing_count: 0,
    }
}

/// ComfyUI 安装状态：镜像是否已就绪 + 权重是否已下载（按下载清单逐文件校验）。
#[tauri::command]
pub async fn comfyui_setup_status(
    app: tauri::AppHandle,
    model_id: String,
    image: String,
    model_files: Option<Vec<String>>,
) -> Result<ComfyuiSetupStatus, AppError> {
    let data_dir = config::get_data_dir(Some(&app))?;
    let weights_dir = data_dir.join("models").join(&model_id);

    let image = image.trim().to_string();
    let image_exists = if image.is_empty() {
        false
    } else {
        image_exists_locally(&image).await
    };

    // 权重完整性：优先按下载清单逐文件核对（能看到「缺哪个文件」），清单不可用时退回目录启发式
    let expected = resolve_expected_files(&app, &weights_dir, model_files.as_deref().unwrap_or(&[])).await;
    let report = match expected.as_deref() {
        Some(files) if !files.is_empty() => inspect_expected_weights(&weights_dir, files),
        _ => inspect_dir_heuristic(&weights_dir),
    };

    Ok(ComfyuiSetupStatus {
        image,
        image_exists,
        weights_dir: weights_dir.to_string_lossy().to_string(),
        weights_downloaded: report.downloaded,
        weights_partial: report.partial,
        weights_bytes: report.bytes,
        weights_total_bytes: report.total,
        weights_missing: report.missing,
        weights_missing_count: report.missing_count,
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
