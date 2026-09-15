//! ComfyUI 服务（「视频生成」页）配套后端：
//! - `comfyui_setup_status`：检查本地镜像是否已构建、权重是否已下载（供页面显示状态）
//! - `build_comfyui_image`：把内置 Dockerfile 写入数据目录并在应用内引导完成 `docker build`
//!   （流式输出到「模型日志」，纯本地构建产物，不走 registry 拉取）
//!
//! 说明：ComfyUI 镜像为 ARM64 自建产物（官方无 ARM64 镜像），因此启动路径只做本地镜像校验，
//! 缺失时提示在「视频生成」页构建，而不是 docker pull。

use crate::bail;
use crate::common::config;
use crate::common::error::AppError;
use crate::common::utils::platform;
use tauri::Emitter;

/// Dockerfile 单一真源：仓库内手工路径 `scripts/docker/h3-comfyui/Dockerfile`
/// （编译期内嵌进二进制，运行时写入 `<data>/build/comfyui/Dockerfile` 再 docker build）
const COMFYUI_DOCKERFILE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scripts/docker/h3-comfyui/Dockerfile"
));

/// 默认 ComfyUI 版本（H3 需 ≥ v0.30.0；Fun ControlNet 模板需 ≥ v0.35.0）
const DEFAULT_COMFYUI_REF: &str = "v0.30.0";

/// 页面上展示的安装状态
#[derive(serde::Serialize)]
pub struct ComfyuiSetupStatus {
    /// 目标镜像名（= 模型清单 engine_image）
    pub image: String,
    /// 本地镜像是否已存在
    pub image_exists: bool,
    /// Dockerfile 构建目录（<data>/build/comfyui）
    pub build_dir: String,
    /// 权重目录（<data>/models/<model_id>）
    pub weights_dir: String,
    /// 权重是否就绪（diffusion_models/ 与 text_encoders/ 下各有 ≥1 个 .safetensors）
    pub weights_downloaded: bool,
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

/// 统计目录下（含一层子目录）指定类型文件的数量与总大小。
fn scan_dir_counts(dir: &std::path::Path, depth: usize) -> (usize, u64) {
    let mut count = 0usize;
    let mut bytes = 0u64;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 1 {
                let (c, b) = scan_dir_counts(&p, depth - 1);
                count += c;
                bytes += b;
            }
            continue;
        }
        let is_model = p
            .extension()
            .map(|x| x.eq_ignore_ascii_case("safetensors") || x.eq_ignore_ascii_case("gguf"))
            .unwrap_or(false);
        if !is_model {
            continue;
        }
        // aria2 预分配会让未完成文件也达到全量长度：有同名 .aria2 控件时视为未完成，不计入
        let aria2 = p.with_file_name(format!(
            "{}.aria2",
            p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        ));
        if aria2.exists() {
            continue;
        }
        if let Ok(m) = e.metadata() {
            count += 1;
            bytes += m.len();
        }
    }
    (count, bytes)
}

/// ComfyUI 安装状态：镜像是否已构建 + 权重是否已下载。
#[tauri::command]
pub async fn comfyui_setup_status(
    app: tauri::AppHandle,
    model_id: String,
    image: String,
) -> Result<ComfyuiSetupStatus, AppError> {
    let data_dir = config::get_data_dir(Some(&app))?;
    let build_dir = data_dir.join("build").join("comfyui");
    let weights_dir = data_dir.join("models").join(&model_id);

    let image = image.trim().to_string();
    let image_exists = if image.is_empty() {
        false
    } else {
        image_exists_locally(&image).await
    };

    let (dit_count, dit_bytes) = scan_dir_counts(&weights_dir.join("diffusion_models"), 2);
    let (enc_count, enc_bytes) = scan_dir_counts(&weights_dir.join("text_encoders"), 2);

    Ok(ComfyuiSetupStatus {
        image,
        image_exists,
        build_dir: build_dir.to_string_lossy().to_string(),
        weights_dir: weights_dir.to_string_lossy().to_string(),
        weights_downloaded: dit_count > 0 && enc_count > 0,
        weights_bytes: dit_bytes + enc_bytes,
    })
}

/// 在应用内构建 ComfyUI 镜像：写入内置 Dockerfile → `docker build -t <image> <dir>`，
/// 输出逐行转发到「模型日志」（model-log 事件），失败时返回可操作的错误信息。
#[tauri::command]
pub async fn build_comfyui_image(
    app: tauri::AppHandle,
    model_id: String,
    image: String,
    comfyui_ref: Option<String>,
) -> Result<(), AppError> {
    let image = image.trim().to_string();
    if image.is_empty() {
        bail!("缺少镜像名（模型清单 engine_image 必填）");
    }
    if image_exists_locally(&image).await {
        let line = format!("[构建] 镜像 {} 已存在本地，跳过构建（如需重建请先 docker rmi）", image);
        crate::common::utils::logger::write_log("INFO", "DOCKER", &line);
        app.emit("model-log", serde_json::json!({
            "model_id": model_id, "line": line, "source": "stdout",
        })).ok();
        return Ok(());
    }

    let data_dir = config::get_data_dir(Some(&app))?;
    let build_dir = data_dir.join("build").join("comfyui");
    std::fs::create_dir_all(&build_dir)
        .map_err(|e| format!("创建构建目录失败 {}: {}", build_dir.display(), e))?;
    let dockerfile = build_dir.join("Dockerfile");
    std::fs::write(&dockerfile, COMFYUI_DOCKERFILE)
        .map_err(|e| format!("写入 Dockerfile 失败 {}: {}", dockerfile.display(), e))?;

    let reference = comfyui_ref
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_COMFYUI_REF)
        .to_string();

    let log = |line: &str, source: &str| {
        crate::common::utils::logger::write_log(
            if source == "stderr" { "WARN" } else { "INFO" },
            "DOCKER",
            &format!("[{}] [build] {}", model_id, line),
        );
        app.emit("model-log", serde_json::json!({
            "model_id": model_id, "line": line, "source": source,
        })).ok();
    };

    crate::pages::model_list::docker_preflight(&app, &model_id).await?;

    log(&format!("[构建] 开始构建镜像 {}（ComfyUI_REF={}）", image, reference), "stdout");
    log(&format!("[构建] Dockerfile: {}", dockerfile.display()), "stdout");

    let mut cmd = platform::docker_cmd_tokio();
    cmd.args([
        "build",
        "-t",
        &image,
        "--build-arg",
        &format!("COMFYUI_REF={}", reference),
        &build_dir.to_string_lossy().to_string(),
    ]);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 docker build 失败: {}", e))?;

    // 逐行转发输出（stdout/stderr 并行读取，避免管道写满阻塞构建）
    use tokio::io::AsyncBufReadExt;
    let mut handles = Vec::new();
    for (is_err, stream) in [
        (false, child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)),
        (true, child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)),
    ] {
        let Some(stream) = stream else { continue };
        let app_c = app.clone();
        let mid = model_id.clone();
        handles.push(tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let truncated: String = line.chars().take(400).collect();
                crate::common::utils::logger::write_log(
                    if is_err { "WARN" } else { "INFO" },
                    "DOCKER",
                    &format!("[{}] [build] {}", mid, truncated),
                );
                app_c.emit("model-log", serde_json::json!({
                    "model_id": mid, "line": truncated, "source": if is_err { "stderr" } else { "stdout" },
                })).ok();
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("等待 docker build 结束失败: {}", e))?;
    if !status.success() {
        let msg = format!(
            "镜像构建失败（退出码 {:?}）：请检查上方构建日志（网络/代理、CUDA 源可达性）后重试",
            status.code()
        );
        log(&format!("[ERROR] {}", msg), "stderr");
        bail!("{}", msg);
    }

    if !image_exists_locally(&image).await {
        let msg = format!("docker build 退出成功但镜像 {} 不可见，请检查 docker CLI 环境", image);
        log(&format!("[ERROR] {}", msg), "stderr");
        bail!("{}", msg);
    }
    log(&format!("[构建] 完成：{} 已就绪", image), "stdout");
    Ok(())
}
