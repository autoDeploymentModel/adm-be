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

/// v0.30.0 对应上游 commit（`git ls-remote --tags` 与 gitee/gitcode/ghfast/gh-proxy 四源核对一致）；
/// 回退到第三方镜像时用于校验克隆结果（git 对象按 SHA 校验，可挡住被篡改的镜像）
const DEFAULT_COMFYUI_REF_SHA: &str = "b1693ecba9f5b65f8c80ab36b195ab963ec92413";

/// ComfyUI 源码候选源（官方优先；github 被 TLS 重置时回退国内镜像/加速器）
const COMFYUI_SOURCE_CANDIDATES: &[&str] = &[
    "https://github.com/comfyanonymous/ComfyUI",
    "https://gitee.com/mirrors/ComfyUI.git",
    "https://gitcode.com/gh_mirrors/co/ComfyUI.git",
    "https://ghfast.top/https://github.com/comfyanonymous/ComfyUI",
    "https://gh-proxy.com/https://github.com/comfyanonymous/ComfyUI",
];

/// 构建基础镜像（Dockerfile `ARG BASE` 默认值，两处需保持一致）
const DEFAULT_BASE_IMAGE: &str = "nvidia/cuda:13.0.0-runtime-ubuntu24.04";

/// 官方源不可达时的内置加速器候选（与设置页「Docker 镜像配置」占位示例一致）；
/// 仅在「已配置加速器」与「官方源」都探测失败后才尝试，避免给可直连网络引入额外绕行。
const FALLBACK_MIRRORS: &[&str] = &[
    "https://docker.nju.edu.cn",
    "https://docker.m.daocloud.io",
    "https://docker.1ms.run",
    "https://docker.1panel.live",
];

/// 单个候选源的元数据探测超时（可达时秒级返回；不可达多为 TCP 超时，提前掐断换下一个）
const BASE_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

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

/// registry-mirrors 条目 → 镜像引用前缀（`https://docker.1ms.run/` → `docker.1ms.run`）
fn mirror_prefix(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let host = trimmed
        .split("://")
        .last()
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    if host.is_empty() || !host.contains('.') {
        return None;
    }
    Some(host.to_string())
}

fn push_unique(out: &mut Vec<String>, value: String) {
    if !out.contains(&value) {
        out.push(value);
    }
}

/// 基础镜像候选引用（保序去重）：已配置加速器 → 官方源 → 内置加速器
fn base_image_candidates(mirrors: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for mirror in mirrors {
        if let Some(prefix) = mirror_prefix(mirror) {
            push_unique(&mut out, format!("{}/{}", prefix, DEFAULT_BASE_IMAGE));
        }
    }
    push_unique(&mut out, DEFAULT_BASE_IMAGE.to_string());
    for mirror in FALLBACK_MIRRORS {
        if let Some(prefix) = mirror_prefix(mirror) {
            push_unique(&mut out, format!("{}/{}", prefix, DEFAULT_BASE_IMAGE));
        }
    }
    out
}

/// 探测镜像引用能否在 registry 侧解析：`docker manifest inspect` 只取元数据、不下载层，
/// 与 BuildKit 构建时的 "load metadata" 走同一条「客户端 → registry」路径，可如实预判是否卡住。
async fn probe_image_ref(reference: &str, proxy: &str) -> bool {
    let mut cmd = platform::docker_cmd_tokio();
    cmd.args(["manifest", "inspect", reference])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if !proxy.is_empty() {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            cmd.env(key, proxy);
        }
        cmd.env("NO_PROXY", "localhost,127.0.0.0/8,::1");
    }
    let Ok(mut child) = cmd.spawn() else {
        return false;
    };
    match tokio::time::timeout(BASE_PROBE_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) => status.success(),
        _ => {
            let _ = child.kill().await;
            false
        }
    }
}

/// 代理是否指向本机（localhost / 127.0.0.1 / ::1）：构建容器需 host 网络才能访问宿主代理
fn proxy_host_is_loopback(proxy: &str) -> bool {
    let after_scheme = proxy.split("://").last().unwrap_or(proxy);
    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    let host_port = host_port.rsplit('@').next().unwrap_or(host_port);
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    matches!(host.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "::1")
}

/// 解析构建用基础镜像：Docker Hub 不可达（国内网络常见 `registry-1.docker.io` i/o timeout，
/// 且 BuildKit 解析 FROM 不读 daemon.json 的 registry-mirrors）时改用可达的加速器前缀版本；
/// 全部不可达则仍返回官方名，由 docker build 报原始错误（报错里给出处理建议）。
async fn resolve_base_image(app: &tauri::AppHandle, log: &impl Fn(&str, &str)) -> String {
    let mirrors = crate::pages::settings::get_docker_mirror_config()
        .await
        .map(|cfg| cfg.mirrors)
        .unwrap_or_default();
    let proxy = crate::common::utils::proxy::proxy_url(app).await;
    for candidate in base_image_candidates(&mirrors) {
        if image_exists_locally(&candidate).await {
            log(&format!("[构建] 基础镜像已存在本地：{}", candidate), "stdout");
            return candidate;
        }
        if probe_image_ref(&candidate, &proxy).await {
            if candidate == DEFAULT_BASE_IMAGE {
                log(&format!("[构建] 基础镜像源可达：{}", candidate), "stdout");
            } else {
                log(
                    &format!("[构建] 官方源不可达，基础镜像改用加速器：{}", candidate),
                    "stdout",
                );
            }
            return candidate;
        }
        log(
            &format!("[构建] 基础镜像源不可达，跳过：{}", candidate),
            "stderr",
        );
    }
    log(
        &format!(
            "[构建] 未探测到可达的基础镜像源，仍按官方源 {} 尝试（可在「设置 → Docker 镜像配置」填加速器或「设置 → 代理」后重试）",
            DEFAULT_BASE_IMAGE
        ),
        "stderr",
    );
    DEFAULT_BASE_IMAGE.to_string()
}

/// 探测 git 源可达性：对 `<repo>/info/refs?service=git-upload-pack` 发一次 GET
/// （git 智能 HTTP 握手第一步，只取引用列表、不传输对象）。
async fn probe_git_repo(url: &str, proxy: &str) -> bool {
    let probe_url = format!(
        "{}/info/refs?service=git-upload-pack",
        url.trim_end_matches('/')
    );
    let mut builder = reqwest::Client::builder()
        .user_agent("git/2.43.0")
        .timeout(std::time::Duration::from_secs(8));
    if !proxy.is_empty() {
        if let Ok(p) = reqwest::Proxy::all(proxy) {
            builder = builder.proxy(p);
        }
    }
    let Ok(client) = builder.build() else {
        return false;
    };
    match client.get(&probe_url).send().await {
        Ok(resp) => resp.status().is_success() || resp.status().is_redirection(),
        Err(_) => false,
    }
}

/// 源码源排序：可达的排前面（不可达的保留在末尾兜底，构建时逐个尝试）。
/// 返回（首选源，其余源）。
async fn resolve_comfyui_sources(log: &impl Fn(&str, &str), proxy: &str) -> (String, Vec<String>) {
    let mut reachable: Vec<String> = Vec::new();
    let mut unreachable: Vec<String> = Vec::new();
    for candidate in COMFYUI_SOURCE_CANDIDATES {
        if probe_git_repo(candidate, proxy).await {
            reachable.push((*candidate).to_string());
        } else {
            unreachable.push((*candidate).to_string());
        }
    }
    if let Some(first) = reachable.first() {
        if first == COMFYUI_SOURCE_CANDIDATES[0] {
            log(&format!("[构建] ComfyUI 源码源可达：{}", first), "stdout");
        } else {
            log(&format!("[构建] 官方源码源不可达，改用：{}", first), "stdout");
        }
    } else {
        log(
            "[构建] 未探测到可达的 ComfyUI 源码源，仍按候选顺序尝试",
            "stderr",
        );
    }
    let mut ordered = reachable;
    ordered.extend(unreachable);
    let first = ordered.remove(0);
    (first, ordered)
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

    // 基础镜像解析（官方源不可达时自动切换加速器前缀）+ 源码源探测 + 代理注入（构建期网络步骤）
    let proxy = crate::common::utils::proxy::proxy_url(&app).await;
    let base_image = resolve_base_image(&app, &log).await;
    let (comfyui_repo, comfyui_repo_fallbacks) = resolve_comfyui_sources(&log, &proxy).await;
    let ref_sha = if reference == DEFAULT_COMFYUI_REF {
        DEFAULT_COMFYUI_REF_SHA.to_string()
    } else {
        String::new()
    };

    log(
        &format!(
            "[构建] 开始构建镜像 {}（COMFYUI_REF={} / BASE={}）",
            image, reference, base_image
        ),
        "stdout",
    );
    log(
        &format!(
            "[构建] ComfyUI 源码：{}（候选回退 {} 个）",
            comfyui_repo,
            comfyui_repo_fallbacks.len()
        ),
        "stdout",
    );
    log(&format!("[构建] Dockerfile: {}", dockerfile.display()), "stdout");

    let mut cmd = platform::docker_cmd_tokio();
    let mut args: Vec<String> = vec![
        "build".to_string(),
        "-t".to_string(),
        image.clone(),
        "--build-arg".to_string(),
        format!("BASE={}", base_image),
        "--build-arg".to_string(),
        format!("COMFYUI_REF={}", reference),
        "--build-arg".to_string(),
        format!("COMFYUI_REPO={}", comfyui_repo),
        "--build-arg".to_string(),
        format!("COMFYUI_REPO_FALLBACKS={}", comfyui_repo_fallbacks.join(" ")),
        "--build-arg".to_string(),
        format!("COMFYUI_REF_SHA={}", ref_sha),
    ];
    if proxy.is_empty() {
        log("[构建] 未配置代理，构建期网络步骤（apt/pip/git）直连", "stdout");
    } else {
        // 代理在本机时构建容器需 host 网络，否则容器内 127.0.0.1 指向容器自身、连不上宿主代理
        if proxy_host_is_loopback(&proxy) {
            args.push("--network=host".to_string());
            log("[构建] 代理在本机：构建期使用 host 网络（--network=host）", "stdout");
        }
        // 构建期网络步骤走同一代理；--build-arg 不写入镜像 ENV（BuildKit 亦不记入镜像历史）
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
        ] {
            let value = if key.eq_ignore_ascii_case("NO_PROXY") {
                "localhost,127.0.0.0/8,::1".to_string()
            } else {
                proxy.clone()
            };
            args.push("--build-arg".to_string());
            args.push(format!("{}={}", key, value));
        }
        log(&format!("[构建] 构建期注入代理：{}", proxy), "stdout");
    }
    args.push(build_dir.to_string_lossy().to_string());
    cmd.args(&args);
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
            "镜像构建失败（退出码 {:?}）：基础镜像 {} 或构建期网络步骤（apt/pip/git）不可达。排查：①「设置 → Docker 镜像配置」填加速器并保存重启 Docker；②「设置 → 代理」配置可用代理（构建期自动注入）；③ 或手动 docker pull {} 后重试。详见上方日志",
            status.code(),
            base_image,
            base_image
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
