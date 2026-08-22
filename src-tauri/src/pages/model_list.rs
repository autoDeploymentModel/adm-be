// model_list.html 对应逻辑（模型管理）

use crate::common::*;
use crate::app_state::AppState;
use crate::common::config;
use crate::common::utils::download::download_with_resume;
use crate::bail;
use crate::dbg_log;

use std::collections::HashMap;
use tauri::Emitter;
use tauri::Manager;

// ===== Tauri Command =====

#[tauri::command]
pub async fn scan_local_models(app: tauri::AppHandle) -> Result<Vec<LocalModel>, AppError> {
    let data_dir = config::get_data_dir(Some(&app))?;
    let models_dir = data_dir.join("models");

    if !models_dir.exists() {
        std::fs::create_dir_all(&models_dir).map_err(|e| format!("创建 models 目录失败: {}", e))?;
        return Ok(Vec::new());
    }

    let mut models = Vec::new();

    for entry in std::fs::read_dir(&models_dir).map_err(|e| format!("读取 models 目录失败: {}", e))?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(dir_name) = path.file_name() {
                let dir_str = dir_name.to_string_lossy().to_string();
                let mut files: Vec<String> = Vec::new();
                if let Ok(dir_entries) = std::fs::read_dir(&path) {
                    for e in dir_entries.flatten() {
                        let fp = e.path();
                        if fp.is_file() {
                            if let Some(name) = fp.file_name() {
                                let name_str = name.to_string_lossy().to_string();
                                if !name_str.ends_with(".part") && name_str != ".done" {
                                    files.push(name_str);
                                }
                            }
                        }
                    }
                }
                if !files.is_empty() {
                    models.push(LocalModel { model_id: dir_str, files });
                }
            }
        } else if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "gguf" {
                    if let Some(stem) = path.file_stem() {
                        let model_id = stem.to_string_lossy().to_string();
                        let filename = path.file_name().unwrap().to_string_lossy().to_string();
                        models.push(LocalModel { model_id, files: vec![filename] });
                    }
                }
            }
        }
    }

    Ok(models)
}

#[tauri::command]
pub async fn scan_part_files(app: tauri::AppHandle) -> Result<Vec<PartFileProgress>, AppError> {
    let data_dir = config::get_data_dir(Some(&app))?;
    let models_dir = data_dir.join("models");

    if !models_dir.exists() {
        return Ok(Vec::new());
    }

    let mut result = Vec::new();

    for entry in std::fs::read_dir(&models_dir).map_err(|e| format!("读取 models 目录失败: {}", e))?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(dir_name) = path.file_name() {
                let dir_str = dir_name.to_string_lossy().to_string();
                let part_file = path.join(format!("{}.gguf.part", dir_str));
                if part_file.exists() {
                    let size = std::fs::metadata(&part_file).map(|m| m.len()).unwrap_or(0);
                    result.push(PartFileProgress { model_id: dir_str.clone(), existing_size: size });
                }
                if let Ok(entries) = std::fs::read_dir(&path) {
                    for entry in entries.flatten() {
                        let fp = entry.path();
                        if fp.is_file() {
                            if let Some(ext) = fp.extension() {
                                if ext == "part" && fp.file_name().is_none_or(|n| n.to_string_lossy() != format!("{}.gguf.part", dir_str).as_str()) {
                                    let size = std::fs::metadata(&fp).map(|m| m.len()).unwrap_or(0);
                                    result.push(PartFileProgress { model_id: dir_str.clone(), existing_size: size });
                                }
                            }
                        }
                    }
                }
            }
        } else if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "part" {
                    if let Some(stem) = path.file_stem() {
                        let stem_str = stem.to_string_lossy().to_string();
                        let model_id = stem_str.trim_end_matches(".gguf").to_string();
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        result.push(PartFileProgress { model_id, existing_size: size });
                    }
                }
            }
        }
    }

    Ok(result)
}

#[tauri::command]
pub async fn fetch_model_list() -> Result<Vec<RemoteModel>, AppError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let response = client
        .get("https://adm.tuduoduo.top/b/model.json")
        .send()
        .await
        .map_err(|e| format!("获取模型列表失败: {}", e))?;

    if !response.status().is_success() {
        bail!("服务器返回错误状态码: {}", response.status());
    }

    let text = response
        .text()
        .await
        .map_err(|e| format!("读取响应文本失败: {}", e))?;

    let models: Vec<RemoteModel> = serde_json::from_str(&text)
        .map_err(|e| format!("解析模型列表失败: {}", e))?;

    Ok(models)
}

#[tauri::command]
pub async fn download_model(
    app: tauri::AppHandle,
    model_id: String,
    model_url: String,
    model_mmproj: Option<String>,
    model_diffusion: Option<String>,
    model_vae: Option<String>,
    model_type: String,
    model_files: Option<Vec<String>>,
) -> Result<(), AppError> {
    {
        let state = app.state::<AppState>();
        let map = state.downloading_progress.lock().map_err(|e| e.to_string())?;
        if map.contains_key(&model_id) {
            bail!("该模型正在下载中，请勿重复点击");
        }
    }

    let data_dir = config::get_data_dir(Some(&app))?;
    let model_dir = data_dir.join("models").join(&model_id);
    std::fs::create_dir_all(&model_dir).map_err(|e| format!("创建模型目录失败: {}", e))?;

    // ===== 新格式：HF 仓库多文件目录下载（safetensors 模型） =====
    if let Some(files) = model_files {
        if !files.is_empty() {
            let total = files.len();
            app.state::<AppState>().downloading_progress.lock().unwrap_or_else(|e| e.into_inner()).insert(model_id.clone(), 0u8);

            struct CleanupGuard2 {
                h: tauri::AppHandle,
                id: String,
            }
            impl Drop for CleanupGuard2 {
                fn drop(&mut self) {
                    if let Ok(mut map) = self.h.state::<AppState>().downloading_progress.lock() {
                        map.remove(&self.id);
                    }
                    if let Ok(mut map) = self.h.state::<AppState>().downloading_phase.lock() {
                        map.remove(&self.id);
                    }
                }
            }
            let _guard = CleanupGuard2 { h: app.clone(), id: model_id.clone() };

            let download_client = reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .build()
                .map_err(|e| format!("创建下载客户端失败: {}", e))?;

            for (idx, url) in files.iter().enumerate() {
                let url = url.replace("https://huggingface.co/", "https://hf-mirror.com/");
                let filename = url
                    .rsplit('/')
                    .next()
                    .unwrap_or(&model_id)
                    .to_string();
                let final_path = model_dir.join(&filename);
                let part_path = model_dir.join(format!("{}.part", filename));

                // 已下载完成的文件直接跳过（断点续传）
                let need_download = if final_path.exists() {
                    // 无 .part 残留即视为完整
                    !part_path.exists()
                } else {
                    true
                };
                if !need_download {
                    app.emit(
                        "download-progress",
                        serde_json::json!({
                            "model_id": &model_id,
                            "progress": ((idx + 1) as f32 * 100.0 / total as f32) as u8,
                            "file": &filename,
                            "type": "model",
                        }),
                    ).ok();
                    continue;
                }

                app.state::<AppState>().downloading_phase.lock().unwrap_or_else(|e| e.into_inner()).insert(model_id.clone(), filename.clone());
                app.emit(
                    "download-progress",
                    serde_json::json!({
                        "model_id": &model_id,
                        "progress": (idx as f32 * 100.0 / total as f32) as u8,
                        "downloaded": 0u64,
                        "total": 0u64,
                        "file": &filename,
                        "type": "model",
                    }),
                ).ok();

                let app_clone = app.clone();
                let mid = model_id.clone();
                let fname = filename.clone();
                let idx_f = idx;
                let total_f = total;
                download_with_resume(
                    &download_client, &url, &final_path, &part_path,
                    |progress, _downloaded, _total| {
                        // 总进度 = 已完成文件 + 当前文件进度折算
                        let overall = ((idx_f as f32 + progress as f32 / 100.0) * 100.0 / total_f as f32) as u8;
                        app_clone.emit(
                            "download-progress",
                            serde_json::json!({
                                "model_id": &mid,
                                "progress": overall,
                                "file": &fname,
                                "type": "model",
                            }),
                        ).ok();
                        if let Ok(mut map) = app_clone.state::<AppState>().downloading_progress.lock() {
                            map.insert(mid.clone(), overall);
                        }
                    },
                ).await?;
                // 单个文件完成不单独发 download-complete（前端收到会清空 downloading
                // 状态导致按钮闪变），进度折算继续由 download-progress 驱动，
                // 全部完成时集中发一次带 all=true 的完成事件。
            }

            // 全部文件下载完成：写 .done 标记（scan_local_models 排除）
            std::fs::write(model_dir.join(".done"), "").ok();
            app.emit(
                "download-complete",
                serde_json::json!({ "model_id": &model_id, "type": "model", "all": true }),
            ).ok();
            return Ok(());
        }
    }

    let model_url = model_url.replace("https://huggingface.co/", "https://hf-mirror.com/");

    let model_filename = model_url
        .rsplit('/')
        .next()
        .unwrap_or(&model_id)
        .to_string();
    let final_path = model_dir.join(&model_filename);
    let part_path = model_dir.join(format!("{}.part", model_filename));

    let download_client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("创建下载客户端失败: {}", e))?;

    app.state::<AppState>().downloading_progress.lock().unwrap_or_else(|e| e.into_inner()).insert(model_id.clone(), 0u8);

    struct CleanupGuard {
        h: tauri::AppHandle,
        id: String,
    }
    impl Drop for CleanupGuard {
        fn drop(&mut self) {
            if let Ok(mut map) = self.h.state::<AppState>().downloading_progress.lock() {
                map.remove(&self.id);
            }
            if let Ok(mut map) = self.h.state::<AppState>().downloading_phase.lock() {
                map.remove(&self.id);
            }
        }
    }
    let _guard = CleanupGuard { h: app.clone(), id: model_id.clone() };

    // ===== 主模型文件下载 =====
    {
        let app_clone = app.clone();
        let mid = model_id.clone();
        download_with_resume(
            &download_client, &model_url, &final_path, &part_path,
            |progress, downloaded, total| {
                app_clone.emit(
                    "download-progress",
                    serde_json::json!({
                        "model_id": &mid,
                        "progress": progress,
                        "downloaded": downloaded,
                        "total": total,
                        "type": "model",
                    }),
                ).ok();
                if let Ok(mut map) = app_clone.state::<AppState>().downloading_progress.lock() {
                    map.insert(mid.clone(), progress);
                }
            },
        ).await?;
        app.emit(
            "download-complete",
            serde_json::json!({ "model_id": &model_id, "type": "model" }),
        ).ok();
    }

    // ===== 视觉多模态：mmproj 文件下载 =====
    if model_type == "视觉多模态理解" {
        if let Some(mmproj_url) = model_mmproj {
            app.state::<AppState>().downloading_phase.lock().unwrap_or_else(|e| e.into_inner()).insert(model_id.clone(), "mmproj".to_string());
            download_extra_file(
                &app, &model_id, &model_dir, &mmproj_url,
                &download_client, "mmproj"
            ).await?;
        }
    }

    // ===== 文生图：diffusion + vae 文件下载 =====
    if model_type == "文本生成图片" {
        if let Some(diffusion_url) = model_diffusion {
            app.state::<AppState>().downloading_phase.lock().unwrap_or_else(|e| e.into_inner()).insert(model_id.clone(), "diffusion".to_string());
            download_extra_file(
                &app, &model_id, &model_dir, &diffusion_url,
                &download_client, "diffusion"
            ).await?;
        }
        if let Some(vae_url) = model_vae {
            app.state::<AppState>().downloading_phase.lock().unwrap_or_else(|e| e.into_inner()).insert(model_id.clone(), "vae".to_string());
            download_extra_file(
                &app, &model_id, &model_dir, &vae_url,
                &download_client, "vae"
            ).await?;
        }
    }

    Ok(())
}

async fn download_extra_file(
    app: &tauri::AppHandle,
    model_id: &str,
    model_dir: &std::path::Path,
    file_url: &str,
    download_client: &reqwest::Client,
    file_type: &str,
) -> Result<(), AppError> {
    let file_url = file_url.replace("https://huggingface.co/", "https://hf-mirror.com/");

    let filename = file_url
        .rsplit('/')
        .next()
        .unwrap_or(file_type)
        .to_string();
    let final_path = model_dir.join(&filename);
    let part_path = model_dir.join(format!("{}.part", filename));

    // 发送初始进度（0%）
    app.emit(
        "download-progress",
        serde_json::json!({
            "model_id": model_id,
            "progress": 0u8,
            "downloaded": 0u64,
            "total": 0u64,
            "type": file_type,
        }),
    )
    .ok();

    // 使用通用下载函数（带断点续传）
    let app_clone = app.clone();
    let mid = model_id.to_string();
    let ft = file_type.to_string();
    download_with_resume(
        download_client, &file_url, &final_path, &part_path,
        |progress, downloaded, total| {
            app_clone.emit(
                "download-progress",
                serde_json::json!({
                    "model_id": &mid,
                    "progress": progress,
                    "downloaded": downloaded,
                    "total": total,
                    "type": &ft,
                }),
            )
            .ok();
        },
    )
    .await?;

    app.emit(
        "download-complete",
        serde_json::json!({ "model_id": model_id, "type": file_type }),
    )
    .ok();

    Ok(())
}

/// Docker 环境预检：CLI 存在 → daemon 运行 → NVIDIA runtime 可用 → 镜像存在（缺失自动拉取，
/// 拉取失败自动回退国内镜像源前缀）。成功返回**实际可用的镜像名**（可能是镜像源前缀版本，
/// 后续 docker run 必须用它）；任一环节失败返回错误原因，前端以 toast / model-log 展示。
async fn check_docker_env(
    app: &tauri::AppHandle,
    model_id: &str,
    image: &str,
) -> Result<String, AppError> {
    let log = |line: String| {
        app.emit(
            "model-log",
            serde_json::json!({
                "model_id": model_id,
                "line": line,
                "source": "stdout",
            }),
        )
        .ok();
    };

    // 1. docker CLI 是否存在
    let cli = crate::common::utils::platform::docker_cmd_tokio()
        .arg("--version")
        .output()
        .await;
    match cli {
        Ok(out) if out.status.success() => {
            log(format!("[Docker] CLI 可用: {}", String::from_utf8_lossy(&out.stdout).trim()));
        }
        _ => {
            return Err(AppError::msg(
                "未检测到 Docker CLI，请先安装 Docker（如 sudo apt install docker.io 或 docker-ce）".to_string(),
            ));
        }
    }

    // 2. docker daemon 是否运行 + 权限检测
    let info = crate::common::utils::platform::docker_cmd_tokio()
        .args(["info"])
        .output()
        .await
        .map_err(|e| AppError::msg(format!("docker info 执行失败: {}", e)))?;
    if !info.status.success() {
        let stderr = String::from_utf8_lossy(&info.stderr).to_string();
        // 权限不足：stderr 含 "permission denied" 或 "Cannot connect to the Docker daemon"
        if stderr.contains("permission denied") || stderr.contains("access denied") {
            return Err(AppError::msg(
                "DOCKER_PERMISSION_DENIED".to_string(),
            ));
        }
        return Err(AppError::msg(
            "Docker daemon 未运行或不可访问，请先启动 Docker 服务（systemd: sudo systemctl start docker；桌面版: 打开 Docker Desktop）".to_string(),
        ));
    }
    let info_text = String::from_utf8_lossy(&info.stdout).to_string();
    // GPU 直通检测：传统 nvidia runtime（daemon.json 配置）或 CDI 模式（/etc/cdi 挂载）任一存在即视为可用
    let has_nvidia_runtime = info_text.contains("nvidia");
    let has_cdi = info_text.contains("CDI");
    log(format!(
        "[Docker] daemon 运行正常; NVIDIA runtime: {}; CDI: {}",
        if has_nvidia_runtime { "可用" } else { "未配置" },
        if has_cdi { "可用" } else { "未配置" }
    ));

    // 3. 镜像检查；已存在则直接可用
    let inspect = crate::common::utils::platform::docker_cmd_tokio()
        .args(["image", "inspect", image])
        .output()
        .await
        .map_err(|e| AppError::msg(format!("docker image inspect 执行失败: {}", e)))?;
    if inspect.status.success() {
        log(format!("[Docker] 镜像 {} 已存在", image));
        return Ok(image.to_string());
    }

    // 4. 镜像不存在 → 拉取。国内镜像源优先，Docker Hub 直连兜底。
    let has_registry_prefix = image
        .split('/')
        .next()
        .map(|first| first.contains('.') || first.contains(':'))
        .unwrap_or(false);
    let mut attempts: Vec<String> = vec![];
    if !has_registry_prefix {
        for mirror in ["docker.1ms.run", "docker.m.daocloud.io", "docker.xuanyuan.me", "hub.rat.dev"] {
            attempts.push(format!("{}/{}", mirror, image));
        }
    }
    attempts.push(image.to_string());

    for (idx, candidate) in attempts.iter().enumerate() {
        let label = if idx == attempts.len() - 1 { "Docker Hub 直连" } else { &format!("镜像源 {}", attempts[idx].split('/').next().unwrap_or("")) };
        log(format!(
            "[Docker] 镜像 {} 不存在（{}），开始{}（首次可能需数分钟）...",
            image, candidate, label
        ));
        // 单源超时熔断：PULL_TIMEOUT 内未完成（卡住/无数据/超慢）则 kill 该 pull 进程，自动切换下一个源
        match pull_image(app, model_id, candidate, PULL_TIMEOUT).await {
            Ok(true) => {
                log(format!("[Docker] 镜像 {} 拉取完成，后续将使用 {}", candidate, candidate));
                return Ok(candidate.clone());
            }
            Ok(false) => {
                log(format!("[Docker] {} 失败，尝试下一个来源...", label));
            }
            Err(e) => {
                log(format!("[Docker] {} 中止: {}，尝试下一个来源...", label, e));
            }
        }
    }

    Err(AppError::msg(format!(
        "镜像拉取失败（已尝试 Docker Hub 与多个国内镜像源，单源连续 {} 分钟无输出即放弃）：{}\n请检查网络后手动执行: docker pull {}；\n或配置镜像加速器：在 /etc/docker/daemon.json 添加 registry-mirrors 后 sudo systemctl restart docker",
        PULL_TIMEOUT.as_secs() / 60,
        image,
        image
    )))
}

/// 单源镜像拉取「闲置」超时（3 分钟）：仅当 stdout/stderr 连续 3 分钟无任何输出
/// （下载进度停滞/连接卡死）才强制终止并切换下一来源；只要进度在动就永不超时。
const PULL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3 * 60);

/// 当前 Unix 毫秒时间戳（闲置超时计算用）
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 拉取单个镜像：stdout/stderr 逐行转发到 model-log，进度按 `\r` 段解析后发
/// `model-pull-progress` 事件（{ model_id, image, progress } 0-100）；成功返回 true，
/// 超时返回 Err（内部已 kill 子进程）。
async fn pull_image(
    app: &tauri::AppHandle,
    model_id: &str,
    image: &str,
    timeout: std::time::Duration,
) -> Result<bool, AppError> {
    let mut child = crate::common::utils::platform::docker_cmd_tokio()
        .args(["pull", image])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| AppError::msg(format!("docker pull 启动失败: {}", e)))?;

    use tokio::io::{AsyncBufReadExt, BufReader};
    use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // 进度估算（非 TTY 下 docker pull 无统一百分比）：
    //   层维度：Pulling fs layer / Pull complete 计数跳格；
    //   字节维度：Downloading / Extracting 行的「当前/总字节」在当前层槽位内插值 → 连续移动
    let total_layers = Arc::new(AtomicUsize::new(0));
    let completed_layers = Arc::new(AtomicUsize::new(0));
    // layer 槽位：sha → 0-based 序号（按出现顺序登记，字节插值用）
    let layer_order: Arc<Mutex<HashMap<String, usize>>> = Arc::new(Mutex::new(HashMap::new()));
    // 已上报的最高进度：docker 并行下载层完成可能乱序，防止进度条回退
    let last_pct = Arc::new(AtomicU8::new(0));

    let emit_progress = |p: u8| {
        app.emit(
            "model-pull-progress",
            serde_json::json!({
                "model_id": model_id,
                "image": image,
                "progress": p,
            }),
        )
        .ok();
    };
    emit_progress(0);

    // 闲置超时计时基准：任一输出行到达即刷新时间戳（下载进度在动 = 永不超时）
    let last_activity = Arc::new(AtomicU64::new(now_millis()));

    let app_c = app.clone();
    let mid = model_id.to_string();
    let _img = image.to_string();

    // stdout：逐行读取（非 TTY 下 docker pull 进度走 stdout）
    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout).lines();
        let app_c2 = app_c.clone();
        let mid2 = mid.clone();
        let total2 = total_layers.clone();
        let done2 = completed_layers.clone();
        let order2 = layer_order.clone();
        let lp2 = last_pct.clone();
        let act = last_activity.clone();
        tokio::spawn(async move {
            while let Ok(Some(line)) = reader.next_line().await {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    act.store(now_millis(), Ordering::Relaxed);
                    crate::common::utils::logger::write_log("INFO", "DOCKER", &format!("[{}] {}", mid2, trimmed));
                    app_c2
                        .emit("model-log", serde_json::json!({
                            "model_id": &mid2, "line": format!("[docker pull] {}", trimmed), "source": "stdout",
                        })).ok();

                    // "<sha>: <状态>" 形式行；sha 为空（纯文本行）则跳过进度统计
                    let (sha, rest) = match trimmed.split_once(':') {
                        Some((s, r)) => (s.trim().to_string(), r.trim_start()),
                        None => (String::new(), ""),
                    };

                    // 层开始：登记总层数并按出现顺序分配槽位
                    if trimmed.contains("Pulling fs layer") || trimmed.contains("Already exists") {
                        let idx = total2.fetch_add(1, Ordering::Relaxed);
                        if !sha.is_empty() {
                            order2.lock().unwrap_or_else(|e| e.into_inner()).entry(sha.clone()).or_insert(idx);
                        }
                    }

                    // 层完成：按完成数跳一格（单调不回落）
                    if trimmed.contains("Pull complete") || trimmed.contains("Already exists") {
                        let done = done2.fetch_add(1, Ordering::Relaxed) + 1;
                        let total = total2.load(Ordering::Relaxed);
                        if total > 0 {
                            let pct = ((done as f64 / total as f64) * 99.0) as u8;
                            let prev = lp2.fetch_max(pct, Ordering::Relaxed);
                            if pct > prev {
                                app_c2.emit("model-pull-progress", serde_json::json!({
                                    "model_id": &mid2, "image": "", "progress": pct,
                                })).ok();
                            }
                        }
                    }

                    // 下载/解压过程行（含 当前/总 字节）：在所在槽位内按字节插值 → 进度连续移动
                    if rest.starts_with("Downloading") || rest.starts_with("Extracting") {
                        if let Some(p) = parse_pull_percent(trimmed) {
                            if p > 0 {
                                let total = total2.load(Ordering::Relaxed);
                                let idx = order2.lock().unwrap_or_else(|e| e.into_inner()).get(&sha).copied().unwrap_or(0);
                                if total > 0 && idx < total {
                                    let pct = (((idx as f64) + (p as f64 / 100.0)) / (total as f64) * 99.0) as u8;
                                    let prev = lp2.fetch_max(pct.min(99), Ordering::Relaxed);
                                    if pct > prev {
                                        app_c2.emit("model-pull-progress", serde_json::json!({
                                            "model_id": &mid2, "image": "", "progress": pct.min(99),
                                        })).ok();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    // stderr：逐行转发（错误信息走 stderr）
    if let Some(stderr) = child.stderr.take() {
        let mut reader = BufReader::new(stderr).lines();
        let app_c3 = app_c.clone();
        let mid3 = mid.clone();
        let act3 = last_activity.clone();
        tokio::spawn(async move {
            while let Ok(Some(line)) = reader.next_line().await {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    act3.store(now_millis(), Ordering::Relaxed);
                    crate::common::utils::logger::write_log("WARN", "DOCKER", &format!("[{}] {}", mid3, trimmed));
                    app_c3
                        .emit("model-log", serde_json::json!({
                            "model_id": &mid3, "line": format!("[docker pull] {}", trimmed), "source": "stderr",
                        })).ok();
                }
            }
        });
    }

    // 闲置超时熔断：仅当连续 timeout 无任何输出（进度停滞/卡死）才终止，
    // 由调用方（check_docker_env 多源回退循环）切换到下一个来源；进度在动则永远等待
    tokio::select! {
        status = child.wait() => {
            let status = status.map_err(|e| AppError::msg(format!("docker pull 等待失败: {}", e)))?;
            let ok = status.success();
            emit_progress(if ok { 100 } else { 0 });
            Ok(ok)
        }
        _ = async {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let idle = now_millis().saturating_sub(last_activity.load(Ordering::Relaxed));
                if idle >= timeout.as_millis() as u64 {
                    break;
                }
            }
        } => {
            // 终止 pull 进程（kill 整棵进程树，含 docker CLI 派生的下载子进程）
            let _ = child.start_kill();
            let _ = child.wait().await;
            emit_progress(0);
            Err(AppError::msg(format!(
                "镜像拉取连续 {} 分钟无任何输出，判定卡死已终止（下载进度在动时不会超时）",
                timeout.as_secs() / 60
            )))
        }
    }
}

/// 解析 docker pull 进度行中的下载百分比。
/// 支持 `123.4MB/512.3MB`、`45KB/2.3MB(KB/MB/GB)` 形式（单位必须一致才计算，
/// 不一致时返回 0，避免误跳进度）。
fn parse_pull_percent(raw: &str) -> Option<u8> {
    let line = raw.replace('\r', "").replace('\n', "");
    let slash = line.find('/')?;
    let before = &line[..slash];
    let after = &line[slash + 1..];

    let parse_amt = |s: &str| -> Option<(f64, u32)> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let mut num_end = 0;
        for (i, c) in s.char_indices() {
            if c.is_ascii_digit() || c == '.' {
                num_end = i + c.len_utf8();
            } else {
                break;
            }
        }
        if num_end == 0 {
            return None;
        }
        let num: f64 = s[..num_end].parse().ok()?;
        let rest = s[num_end..].trim();
        // 单位：KB/MB/GB/TB（取首字母大写，按指数换算）
        let unit = rest.chars().next()?;
        let exp = match unit.to_ascii_uppercase() {
            'K' => 1u32,
            'M' => 2u32,
            'G' => 3u32,
            'T' => 4u32,
            _ => return None,
        };
        Some((num, exp))
    };

    // 取 "/" 前最后一个数字段，"/" 后第一个数字段
    let bf = before.split_whitespace().last()?;
    let (num_b, exp_b) = parse_amt(bf)?;
    let (num_a, exp_a) = parse_amt(after)?;
    if exp_a != exp_b || num_a <= 0.0 {
        return Some(0);
    }
    let pct = ((num_b / num_a) * 100.0).round() as u8;
    Some(pct.min(99))
}

/// SGLang Docker 启动（Ubuntu / DGX Spark 等机型）。
/// 模型目录以只读方式挂载进容器，容器前台运行（生命周期 = docker run 进程），
/// 就绪信号：stdout 出现 "Uvicorn running on"（SGLang 启动完成）。
async fn start_sglang_docker(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
    model_id: &str,
    model_dir: &std::path::Path,
    params: LaunchParams,
    device: Option<String>,
    sglang_version: Option<String>,
) -> Result<(), AppError> {
    const CONTAINER_PREFIX: &str = "adm-sglang-";
    let container_name = format!("{}{}", CONTAINER_PREFIX, model_id);

    // 加载设置中的 SGLang 详细参数配置
    let settings_path = config::get_data_dir(Some(app))?.join("config.json");
    let mut sglang_args = SglangArgs::default();
    if let Ok(json) = std::fs::read_to_string(&settings_path) {
        if let Ok(parsed) = serde_json::from_str::<Settings>(&json) {
            sglang_args = parsed.sglang_args;
        }
    }

    // 机型 → 启动配置（后续按机型扩展）；镜像/内存以设置页配置为准（缺省按机型兜底）
    let default_image = match device.as_deref() {
        Some("dgx-spark-128G") => "lmsysorg/sglang:v0.5.17",
        _ => "lmsysorg/sglang:v0.5.17",
    };
    let default_shm = match device.as_deref() {
        Some("dgx-spark-128G") => "64g",
        _ => "32g",
    };
    // 镜像优先级：模型配置 sglang-version > 设置页镜像 > 机型默认。
    // 版本字段直接写 tag（如 "v0.5.17"）则拼 lmsysorg/sglang:；写完整镜像名（含 "/" 或 lmsysorg/ 前缀）则原样使用
    let mut image = if sglang_args.image.is_empty() { default_image.to_string() } else { sglang_args.image.clone() };
    if let Some(ver) = sglang_version.as_deref().map(str::trim) {
        if !ver.is_empty() {
            image = if ver.contains('/') || ver.starts_with("lmsysorg/") {
                ver.to_string()
            } else {
                format!("lmsysorg/sglang:{}", ver)
            };
        }
    }
    crate::common::utils::logger::write_log("INFO", "DOCKER", &format!("[{}] 使用镜像 {}（优先级：模型 sglang-version > 设置页 > 默认）", model_id, image));
    app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("使用镜像 {}（模型 sglang-version / 设置页 / 默认）", image),
        "source": "stdout",
    })).ok();
    let shm_size = if sglang_args.shm_size.is_empty() { default_shm.to_string() } else { sglang_args.shm_size.clone() };

    // ===== 启动前 Docker 环境预检（CLI / daemon / GPU runtime / 镜像）=====
    // 返回实际可用镜像名（可能是国内镜像源前缀版本），后续 docker run 必须用它
    let image = check_docker_env(app, model_id, &image).await?;

    let port: u16 = params.port.unwrap_or(5678);
    // Docker 容器内必须监听 0.0.0.0 才能经 -p 端口映射对外服务；设置页 host 不适用容器内
    let host = "0.0.0.0".to_string();

    // 端口占用检查：镜像就绪后、容器启动前，先确认宿主机端口可 bind（避免启动即端口冲突退出）
    {
        let probe = std::net::TcpListener::bind(("0.0.0.0", port));
        if probe.is_err() {
            bail!(
                "端口 {} 已被占用，请先关闭占用该端口的进程，或在设置页更换监听端口",
                port
            );
        }
    }

    // 清理同名残留容器
    let _ = crate::common::utils::platform::docker_cmd()
        .args(["rm", "-f", &container_name])
        .output();

    let mount_src = model_dir.to_string_lossy().to_string();
    let mount_dst = format!("/models/{}", model_id);

    let mut args: Vec<String> = vec![
        "run".to_string(),
        "-e".to_string(),
        "PYTHONWARNINGS=ignore::FutureWarning".to_string(),
        "--name".to_string(),
        container_name.clone(),
        "--gpus".to_string(),
        "all".to_string(),
        "--shm-size".to_string(),
        shm_size,
        "--cap-add".to_string(),
        "SYS_NICE".to_string(),
        "--ipc".to_string(),
        "host".to_string(),
        "-p".to_string(),
        format!("{}:{}", port, port),
        "-v".to_string(),
        format!("{}:{}:ro", mount_src, mount_dst),
        image,
        "python3".to_string(),
        "-m".to_string(),
        "sglang.launch_server".to_string(),
        "--model-path".to_string(),
        mount_dst,
        "--host".to_string(),
        host,
        "--port".to_string(),
        port.to_string(),
    ];

    // ===== 设置页 SGLang 详细参数（仅非空/非默认值才追加） =====
    // 上下文大小：设置页 ctx_size 优先，其次 sglang_args.context_length
    let mut ctx: i64 = params.ctx_size.unwrap_or(0) as i64;
    if ctx <= 0 { ctx = sglang_args.context_length; }
    if ctx > 0 {
        args.extend(["--context-length".to_string(), ctx.to_string()]);
    }
    if sglang_args.tensor_parallel_size > 1 {
        args.extend(["--tensor-parallel-size".to_string(), sglang_args.tensor_parallel_size.to_string()]);
    }
    if sglang_args.mem_fraction_static > 0.0 {
        args.extend(["--mem-fraction-static".to_string(), format!("{}", sglang_args.mem_fraction_static)]);
    }
    if !sglang_args.dtype.is_empty() {
        args.extend(["--dtype".to_string(), sglang_args.dtype.clone()]);
    }
    if !sglang_args.quantization.is_empty() {
        args.extend(["--quantization".to_string(), sglang_args.quantization.clone()]);
    }
    if !sglang_args.kv_cache_dtype.is_empty() {
        args.extend(["--kv-cache-dtype".to_string(), sglang_args.kv_cache_dtype.clone()]);
    }
    if !sglang_args.schedule_policy.is_empty() {
        args.extend(["--schedule-policy".to_string(), sglang_args.schedule_policy.clone()]);
    }
    if sglang_args.max_running_requests > 0 {
        args.extend(["--max-running-requests".to_string(), sglang_args.max_running_requests.to_string()]);
    }
    if sglang_args.max_queued_requests > 0 {
        args.extend(["--max-queued-requests".to_string(), sglang_args.max_queued_requests.to_string()]);
    }
    if sglang_args.chunked_prefill_size != 0 {
        args.extend(["--chunked-prefill-size".to_string(), sglang_args.chunked_prefill_size.to_string()]);
    }
    if !sglang_args.log_level.is_empty() && sglang_args.log_level != "info" {
        args.extend(["--log-level".to_string(), sglang_args.log_level.clone()]);
    }
    if sglang_args.log_requests {
        args.push("--log-requests".to_string());
    }
    if sglang_args.enable_metrics {
        args.push("--enable-metrics".to_string());
    }
    if !sglang_args.reasoning_parser.is_empty() {
        args.extend(["--reasoning-parser".to_string(), sglang_args.reasoning_parser.clone()]);
    }
    if !sglang_args.tool_call_parser.is_empty() {
        args.extend(["--tool-call-parser".to_string(), sglang_args.tool_call_parser.clone()]);
    }
    // 额外参数：每行一个 key=value，拼成 --key value
    for line in sglang_args.extra_args.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim().trim_start_matches("--");
            let v = v.trim();
            if !k.is_empty() && !v.is_empty() {
                args.push(format!("--{}", k));
                args.push(v.to_string());
            }
        }
    }

    // ===== MTP（Multi-Token Prediction）自动启用 =====
    // 模型目录含 MTP 权重（如 model_mtp.safetensors）时自动启用 NEXTN 投机解码
    // （SGLang 中 NEXTN 是 EAGLE 的别名；MTP 权重与主模型同目录，SGLang 自动加载，
    // 无需 --speculative-draft-model-path。参考 DeepSeek-V3.2 官方用法：
    // --speculative-algorithm EAGLE --speculative-num-steps 3
    // --speculative-eagle-topk 1 --speculative-num-draft-tokens 4）
    // 用户已在额外参数里自定义 speculative 相关参数时跳过（尊重覆盖，
    // 也可用 --speculative-algorithm NONE 显式关闭）。
    let has_mtp_weight = {
        let mut found = false;
        if let Ok(entries) = std::fs::read_dir(model_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.contains("mtp") && (name.ends_with(".safetensors") || name.ends_with(".bin")) {
                    found = true;
                    break;
                }
            }
        }
        found
    };
    let user_specified_spec = sglang_args.extra_args.lines().any(|l| {
        let t = l.trim();
        t.starts_with("--speculative-algorithm") || t.starts_with("speculative-algorithm")
    });
    if has_mtp_weight && !user_specified_spec {
        args.extend([
            "--speculative-algorithm".to_string(),
            "EAGLE".to_string(),
            "--speculative-num-steps".to_string(),
            "3".to_string(),
            "--speculative-eagle-topk".to_string(),
            "1".to_string(),
            "--speculative-num-draft-tokens".to_string(),
            "4".to_string(),
        ]);
        app.emit(
            "model-log",
            serde_json::json!({
                "model_id": model_id,
                "line": "[MTP] 检测到 MTP 权重，已自动启用 EAGLE 投机解码（num-steps=3, eagle-topk=1, num-draft-tokens=4）",
                "source": "stdout",
            }),
        )
        .ok();
        crate::common::utils::logger::write_log("INFO", "MODEL", &format!("[{}] MTP 权重检测到，已启用 EAGLE 投机解码", model_id));
    }

    dbg_log!("[DEBUG] sglang docker args: {:?}", args);

    app.emit(
        "model-log",
        serde_json::json!({
            "model_id": model_id,
            "line": format!("[DEBUG] full command: docker {:?}", args),
            "source": "stdout",
        }),
    )
    .ok();

    #[cfg(target_os = "windows")]
    let mut child = crate::common::utils::platform::docker_cmd()
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            let msg = format!("启动推理引擎容器失败: {}", e);
            app.emit("model-log", serde_json::json!({
                "model_id": model_id, "line": format!("[ERROR] {}", msg), "source": "stderr",
            })).ok();
            msg
        })?;

    #[cfg(not(target_os = "windows"))]
    let mut child = crate::common::utils::platform::spawn_detached(
        crate::common::utils::platform::docker_cmd().args(&args).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()),
    )
    .map_err(|e| {
        let msg = format!("启动推理引擎容器失败: {}", e);
        app.emit("model-log", serde_json::json!({
            "model_id": model_id, "line": format!("[ERROR] {}", msg), "source": "stderr",
        })).ok();
        msg
    })?;

    let pid = child.id();

    {
        let mut pid_lock = state.running_process.lock().map_err(|e| e.to_string())?;
        *pid_lock = Some(pid);
    }
    {
        let mut model_lock = state.running_model_id.lock().map_err(|e| e.to_string())?;
        *model_lock = Some(model_id.to_string());
    }
    {
        let mut port_lock = state.running_port.lock().map_err(|e| e.to_string())?;
        *port_lock = Some(port);
    }
    {
        let mut container_lock = state.running_container.lock().map_err(|e| e.to_string())?;
        *container_lock = Some(container_name.clone());
    }
    state.set_model_running(true);
    state.bump_model_generation();

    app.emit(
        "model-started",
        serde_json::json!({ "model_id": model_id, "port": port }),
    )
    .ok();

    let app_clone = app.clone();
    let model_id_clone = model_id.to_string();
    let app_clone2 = app.clone();
    let model_id_clone2 = model_id.to_string();

    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};

        let stdout_handle = if let Some(stdout) = child.stdout.take() {
            let app_c = app_clone.clone();
            let mid = model_id_clone.clone();
            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    crate::common::utils::logger::write_log("INFO", "SGLang", &line);
                    app_c
                        .emit("model-log", serde_json::json!({
                            "model_id": &mid, "line": line.clone(), "source": "stdout",
                        }))
                        .ok();
                    // 推理引擎就绪信号
                    if line.contains("Uvicorn running on")
                        || line.contains("The server is fired up and ready to rock!")
                    {
                        app_c
                            .emit("model-started", serde_json::json!({
                                "model_id": &mid, "port": port,
                            }))
                            .ok();
                    }
                }
            }))
        } else {
            None
        };

        let stderr_handle = if let Some(stderr) = child.stderr.take() {
            let app_c = app_clone.clone();
            let mid = model_id_clone.clone();
            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    crate::common::utils::logger::write_log("WARN", "SGLang", &line);
                    app_c
                        .emit("model-log", serde_json::json!({
                            "model_id": &mid, "line": line, "source": "stderr",
                        }))
                        .ok();
                }
            }))
        } else {
            None
        };

        if let Some(h) = stdout_handle { let _ = h.join(); }
        if let Some(h) = stderr_handle { let _ = h.join(); }

        let exit_status = child.wait();
        match &exit_status {
            Ok(status) => {
                app_clone2.emit("model-log", serde_json::json!({
                    "model_id": &model_id_clone2,
                    "line": format!("[DEBUG] 推理引擎容器退出 with status: {}", status),
                    "source": "stdout",
                })).ok();
            }
            Err(e) => {
                app_clone2.emit("model-log", serde_json::json!({
                    "model_id": &model_id_clone2,
                    "line": format!("[ERROR] 推理引擎容器等待失败: {}", e),
                    "source": "stderr",
                })).ok();
            }
        }

        // 容器退出：清理容器与状态
        let _ = crate::common::utils::platform::docker_cmd()
            .args(["rm", "-f", &container_name])
            .output();

        {
            let state = app_clone2.state::<AppState>();
            *state.running_process.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *state.running_model_id.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *state.running_port.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *state.running_container.lock().unwrap_or_else(|e| e.into_inner()) = None;
            state.set_model_running(false);
        }

        app_clone2
            .emit("model-stopped", serde_json::json!({ "model_id": &model_id_clone2 }))
            .ok();
    });

    Ok(())
}

#[tauri::command]
pub async fn start_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    model_id: String,
    params: LaunchParams,
    device: Option<String>,
    sglang_version: Option<String>,
) -> Result<(), AppError> {
    {
        let pid_lock = state.running_process.lock().map_err(|e| e.to_string())?;
        if pid_lock.is_some() {
            bail!("已有模型在运行中，请先停止当前模型");
        }
    }

    let data_dir = config::get_data_dir(Some(&app))?;
    let models_dir = data_dir.join("models");
    let model_dir = models_dir.join(&model_id);

    // ===== 新格式（safetensors 目录模型）：SGLang Docker 启动 =====
    let is_dir_model = model_dir.join(".done").exists()
        || (model_dir.join("config.json").exists() && model_dir.join("model.safetensors").exists());
    if is_dir_model {
        return start_sglang_docker(&app, &state, &model_id, &model_dir, params, device, sglang_version).await;
    }

    // 仅支持 SGLang Docker 部署（safetensors 目录模型）
    Err(AppError::msg("当前仅支持推理引擎（safetensors 目录）模型，请下载新版模型后重试"))
}

#[tauri::command]
pub async fn stop_model(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    let pid = {
        let pid_lock = state.running_process.lock().map_err(|e| e.to_string())?;
        pid_lock.ok_or("没有正在运行的模型")?
    };
    // SGLang Docker 模式：先优雅停止容器，再兜底杀进程
    let container = state.running_container.lock().map_err(|e| e.to_string())?.clone();
    if let Some(container_name) = container {
        // docker stop/rm 放进 spawn_blocking 并整体限时：docker CLI 卡死（守护进程无响应等）
        // 时最多等待 20s 即放弃，避免"关闭模型"永久无响应
        let timeout = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            tokio::task::spawn_blocking({
                let name = container_name.clone();
                move || {
                    let _ = crate::common::utils::platform::docker_cmd()
                        .args(["stop", "-t", "5", &name])
                        .output();
                    let _ = crate::common::utils::platform::docker_cmd()
                        .args(["rm", "-f", &name])
                        .output();
                }
            }),
        )
        .await;
        if timeout.is_err() {
            crate::common::utils::logger::write_log(
                "WARN",
                "MODEL",
                &format!("[{}] docker stop/rm 超时（20s），改用进程树强杀兜底", container_name),
            );
        }
    }

    crate::common::utils::platform::kill_process_tree(pid);

    {
        let mut pid_lock = state.running_process.lock().map_err(|e| e.to_string())?;
        *pid_lock = None;
    }
    {
        let mut model_lock = state.running_model_id.lock().map_err(|e| e.to_string())?;
        *model_lock = None;
    }
    {
        let mut port_lock = state.running_port.lock().map_err(|e| e.to_string())?;
        *port_lock = None;
    }
    {
        let mut container_lock = state.running_container.lock().map_err(|e| e.to_string())?;
        *container_lock = None;
    }
    state.set_model_running(false);

    Ok(())
}

#[tauri::command]
pub async fn delete_local_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    model_id: String,
) -> Result<(), AppError> {
    {
        let running_id = state.running_model_id.lock().map_err(|e| e.to_string())?;
        if let Some(ref rid) = *running_id {
            if rid == &model_id {
                bail!("模型正在运行中，请先关闭后再删除");
            }
        }
    }

    let data_dir = config::get_data_dir(Some(&app))?;
    let models_dir = data_dir.join("models");

    let dir_path = models_dir.join(&model_id);
    if dir_path.exists() {
        std::fs::remove_dir_all(&dir_path)
            .map_err(|e| format!("删除模型目录失败: {}", e))?;
    }

    let file_path = models_dir.join(format!("{}.gguf", model_id));
    if file_path.exists() {
        std::fs::remove_file(&file_path)
            .map_err(|e| format!("删除模型文件失败: {}", e))?;
    }

    Ok(())
}

#[tauri::command]
pub async fn get_model_status(state: tauri::State<'_, AppState>) -> Result<ModelStatus, AppError> {
    let pid = *state
        .running_process
        .lock()
        .map_err(|e| e.to_string())?;
    let model_id = state
        .running_model_id
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let port = *state.running_port.lock().map_err(|e| e.to_string())?;

    let running = if let Some(pid) = pid {
        let mut sys = sysinfo::System::new();
        sys.refresh_all();
        sys.process(sysinfo::Pid::from_u32(pid)).is_some()
    } else {
        false
    };

    if !running {
        let mut pid_lock = state.running_process.lock().map_err(|e| e.to_string())?;
        *pid_lock = None;
        let mut model_lock = state.running_model_id.lock().map_err(|e| e.to_string())?;
        *model_lock = None;
        let mut port_lock = state.running_port.lock().map_err(|e| e.to_string())?;
        *port_lock = None;
    }

    Ok(ModelStatus {
        running,
        model_id,
        pid,
        port,
    })
}

#[tauri::command]
pub async fn get_downloading_models(state: tauri::State<'_, AppState>) -> Result<HashMap<String, u8>, AppError> {
    let map = state.downloading_progress.lock().map_err(|e| e.to_string())?;
    Ok(map.clone())
}

#[tauri::command]
pub async fn get_downloading_phases(state: tauri::State<'_, AppState>) -> Result<HashMap<String, String>, AppError> {
    let map = state.downloading_phase.lock().map_err(|e| e.to_string())?;
    Ok(map.clone())
}
