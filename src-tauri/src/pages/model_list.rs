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

/// 展开分片文件模式：URL 文件名形如 `model-00001-of-00048.safetensors` 且序号以 1
/// 开头时，自动展开为 model-00001..00048-of-00048；非分片模式原样返回自身。
fn expand_shard_urls(url: &str) -> Vec<String> {
    let Some((dir, fname)) = url.rsplit_once('/') else {
        return vec![url.to_string()];
    };
    let Some(of_idx) = fname.rfind("-of-") else {
        return vec![url.to_string()];
    };
    let (head, tail) = fname.split_at(of_idx);
    let Some(dash) = head.rfind('-') else {
        return vec![url.to_string()];
    };
    let prefix = &head[..dash];
    let start_str = &head[dash + 1..];
    let rest = &tail[4..]; // 跳过 "-of-"
    let Some(dot) = rest.find('.') else {
        return vec![url.to_string()];
    };
    let total_str = &rest[..dot];
    let ext = &rest[dot..];
    if start_str.is_empty()
        || total_str.is_empty()
        || !start_str.bytes().all(|b| b.is_ascii_digit())
        || !total_str.bytes().all(|b| b.is_ascii_digit())
    {
        return vec![url.to_string()];
    }
    let Ok(start) = start_str.parse::<u32>() else {
        return vec![url.to_string()];
    };
    let Ok(total) = total_str.parse::<u32>() else {
        return vec![url.to_string()];
    };
    // 仅当序号从 1 开始才展开全部分片，避免误伤其他命名；
    // 上限 1000 防止恶意/异常 JSON 导致 OOM
    if total <= 1 || total > 1000 || start != 1 {
        return vec![url.to_string()];
    }
    let width = start_str.len();
    (1..=total)
        .map(|i| format!("{}/{}-{:0width$}-of-{}{}", dir, prefix, i, total_str, ext, width = width))
        .collect()
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

    let mut models: Vec<RemoteModel> = serde_json::from_str(&text)
        .map_err(|e| format!("解析模型列表失败: {}", e))?;

    // 统一在此处展开分片 URL，使 fetch_model_list 的返回值与下载清单一致；
    // download_model 和前端均直接使用展开后的结果，无需重复展开
    for m in &mut models {
        if m.model_download_files.is_empty() {
            continue;
        }
        let expanded: Vec<String> = m.model_download_files.iter().flat_map(|u| expand_shard_urls(u)).collect();
        let mut seen = std::collections::HashSet::new();
        m.model_download_files = expanded
            .into_iter()
            .filter(|u| seen.insert(u.clone()))
            .collect();
    }

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
            // 分片 URL 已在 fetch_model_list 中展开，此处直接使用
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
        // 同时写入日志文件（设置 → 运行日志可查），与事件转发并存
        crate::common::utils::logger::write_log("INFO", "DOCKER", &line);
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

    // 4. 镜像不存在 → 直接 docker pull（镜像加速由 daemon.json registry-mirrors
    //    配置全局生效，见设置页「Docker 镜像配置」，不再做应用内镜像源回退）
    match pull_image(app, model_id, image, PULL_TIMEOUT).await {
        Ok(true) => {
            log(format!("[Docker] 镜像 {} 拉取完成", image));
            return Ok(image.to_string());
        }
        Ok(false) => {
            log(format!("[Docker] 拉取 {} 失败，中止启动", image));
        }
        Err(e) => {
            log(format!("[Docker] 拉取 {} 中止: {}", image, e));
        }
    }

    Err(AppError::msg(format!(
        "镜像拉取失败（单源连续 {} 分钟无输出即放弃）：{}\n请检查网络或镜像地址后手动执行: docker pull {}；\n国内网络可改用镜像加速：设置页「Docker 镜像配置」写入加速器地址（如 https://docker.1ms.run）并重启 Docker",
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

// ===== 多机互联（2+ 台 DGX Spark 集群）=====
// 设计文档：doc/dgx-spark-multinode-plan.md。
// 原则：单机路径（start_sglang_docker）零改动；仅当设置页 multi_node_args.enabled 且节点数 >= 2 时走本分支。

/// 前台运行 docker run（多机 rank 0 用）：Windows 直接 spawn，Unix 新建进程组便于整树清理
#[cfg(target_os = "windows")]
fn spawn_docker_run(args: &[String]) -> std::io::Result<std::process::Child> {
    crate::common::utils::platform::docker_cmd()
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
}

#[cfg(not(target_os = "windows"))]
fn spawn_docker_run(args: &[String]) -> std::io::Result<std::process::Child> {
    crate::common::utils::platform::spawn_detached(
        crate::common::utils::platform::docker_cmd()
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()),
    )
}

/// 从 config.json 读取多机配置（不存在/解析失败回默认值）
fn load_multi_node_config(app: &tauri::AppHandle) -> (MultiNodeArgs, SglangArgs) {
    let mut mn = MultiNodeArgs::default();
    let mut sa = SglangArgs::default();
    if let Ok(settings_path) = config::get_data_dir(Some(app)).map(|d| d.join("config.json")) {
        if let Ok(json) = std::fs::read_to_string(settings_path) {
            if let Ok(parsed) = serde_json::from_str::<Settings>(&json) {
                mn = parsed.multi_node_args;
                sa = parsed.sglang_args;
            }
        }
    }
    (mn, sa)
}

/// 多机配置校验（启动前，失败列出具体原因）
fn validate_multi_node(mn: &MultiNodeArgs, port: u16) -> Result<(), AppError> {
    // DGX 直连 v1：固定本机 + 1 台直连节点
    if mn.nodes.len() != 2 {
        bail!("DGX 直连模式固定 2 台机器（本机 + 1 台直连节点），当前配置 {} 台，请在设置页「DGX直连配置」修正", mn.nodes.len());
    }
    if !mn.nodes[0].is_self {
        bail!("节点清单第 1 条（rank 0）必须勾选「本机」");
    }
    if mn.nodes[0].ip.trim().is_empty() {
        bail!("本机（rank 0）IP 不能为空（其他节点需通过该地址互联）");
    }
    if mn.dist_init_port == 0 {
        bail!("引导端口（--dist-init-addr 端口）不能为 0");
    }
    if mn.dist_init_port == port {
        bail!("引导端口 {} 与模型服务端口冲突，请在设置页修改引导端口", port);
    }
    for (i, n) in mn.nodes.iter().enumerate() {
        if n.ip.trim().is_empty() {
            bail!("节点 {}（rank {}）IP 为空", i + 1, i);
        }
        crate::common::ssh::validate_host(&n.ip).map_err(|e| e.to_string())?;
        if !n.is_self {
            crate::common::ssh::validate_ssh_user(&n.ssh_user).map_err(|e| e.to_string())?;
        }
        if !n.is_self && n.ssh_user.trim().is_empty() {
            bail!("节点 {}（rank {}）SSH 用户为空", i + 1, i);
        }
        // 模型目录允许留空：远端自动使用 ~/models/<model_id>
    }
    Ok(())
}

/// 远端模型目录：留空时自动默认 /home/<ssh_user>/models/<model_id>（绝对路径，无波浪号转义问题；
/// SSH 用户为空时回退 ~/models/<model_id>，远端 shell 可展开 ~）。
/// 用户填写时视为模型根目录，追加 /<model_id> 子目录（与 sync_model_to_remote 一致）。
fn effective_remote_model_dir(model_dir: &str, user: &str, model_id: &str) -> String {
    let t = model_dir.trim();
    if t.is_empty() {
        let u = user.trim();
        if u.is_empty() {
            format!("~/models/{}", model_id)
        } else {
            format!("/home/{}/models/{}", u, model_id)
        }
    } else {
        format!("{}/{}", t, model_id)
    }
}

/// 模型目录是否含 MTP 权重（文件名含 mtp 且 .safetensors / .bin）
fn has_mtp_weight(model_dir: &std::path::Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(model_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.contains("mtp") && (name.ends_with(".safetensors") || name.ends_with(".bin")) {
                return true;
            }
        }
    }
    false
}

/// 用户（设置页 extra_args 或模型清单 sglang_flags）是否已自定义 speculative-algorithm
fn spec_user_defined(sglang_args: &SglangArgs, sglang_flags: &[String]) -> bool {
    sglang_args.extra_args.lines().any(|l| {
        let t = l.trim();
        t.starts_with("--speculative-algorithm") || t.starts_with("speculative-algorithm")
    }) || sglang_flags.iter().any(|f| f.contains("speculative-algorithm"))
}

/// 本机通过 IP 反查网卡名：`ip -o -4 addr show to <IP>` 取接口名。
/// 仅 Linux 多机模式调用；Windows 开发环境返回 None（不影响开发）。
fn detect_local_iface(ip: &str) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        let _ = ip;
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        let out = std::process::Command::new("sh")
            .args(["-c", &format!("ip -o -4 addr show to {} 2>/dev/null | awk '{{print $2}}' | head -1", ip)])
            .output()
            .ok()?;
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if name.is_empty() { None } else { Some(name) }
    }
}

/// 远端通过 SSH 反查网卡名：SSH 执行 `ip -o -4 addr show to <IP>`。
async fn detect_remote_iface(
    ip: &str, user: &str, port: u16, key: Option<&str>,
    target_ip: &str,
) -> Option<String> {
    let cmd = format!(
        "ip -o -4 addr show to {} 2>/dev/null | awk '{{print $2}}' | head -1",
        target_ip
    );
    let (ok, stdout, _) = crate::common::ssh::ssh_run(
        ip, user, port, key, &cmd, std::time::Duration::from_secs(10),
    )
    .await
    .ok()?;
    if !ok { return None; }
    let name = stdout.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

/// 构建某 rank 的完整 docker run 参数（多机模式）。
///
/// 与单机的差异：`--network host`（替代 -p 端口映射，NCCL 跨机必需）；
/// 多机参数（--tp N --nnodes N --node-rank R --dist-init-addr）置于命令**最后**，
/// 保证跨节点 TP 拓扑不被设置页/模型清单的同名参数覆盖。
/// 设置页 sglang_args / MTP 自动启用 / 模型清单 sglang_flags 拼接规则与单机一致。
/// `iface`：自动从节点 IP 反查的网卡名，注入 NCCL_SOCKET_IFNAME / GLOO_SOCKET_IFNAME；
///          None 时不注入（NCCL 自动发现）。
fn build_multi_node_args(
    model_id: &str,
    model_dir: &std::path::Path,
    image: &str,
    shm_size: &str,
    port: u16,
    sglang_args: &SglangArgs,
    sglang_flags: &Option<Vec<String>>,
    mn: &MultiNodeArgs,
    rank: usize,
    node0_ip: &str,
    iface: Option<&str>,
) -> Vec<String> {
    let container_name = format!("adm-sglang-{}-rank-{}", model_id, rank);
    let mount_dst = format!("/models/{}", model_id);
    let mut args: Vec<String> = vec![
        "run".to_string(),
        "-e".to_string(),
        "PYTHONWARNINGS=ignore::FutureWarning".to_string(),
        "--name".to_string(),
        container_name,
        "--gpus".to_string(),
        "all".to_string(),
        "--shm-size".to_string(),
        shm_size.to_string(),
        "--cap-add".to_string(),
        "SYS_NICE".to_string(),
        "--ipc".to_string(),
        "host".to_string(),
        "--network".to_string(),
        "host".to_string(),
    ];
    if mn.use_roce {
        args.extend([
            "--device".to_string(),
            "/dev/infiniband".to_string(),
            "--ulimit".to_string(),
            "memlock=-1:-1".to_string(),
            "--cap-add".to_string(),
            "IPC_LOCK".to_string(),
        ]);
    }
    if let Some(iface_name) = iface.filter(|s| !s.trim().is_empty()) {
        args.extend([
            "-e".to_string(),
            format!("NCCL_SOCKET_IFNAME={}", iface_name.trim()),
            "-e".to_string(),
            format!("GLOO_SOCKET_IFNAME={}", iface_name.trim()),
        ]);
    }
    args.push("-v".to_string());
    args.push(format!(
        "{}:{}{}",
        model_dir.to_string_lossy(),
        mount_dst,
        ":ro"
    ));
    args.push(image.to_string());
    args.push("python3".to_string());
    args.push("-m".to_string());
    args.push("sglang.launch_server".to_string());
    args.push("--model-path".to_string());
    args.push(mount_dst);
    args.push("--host".to_string());
    args.push("0.0.0.0".to_string());
    args.push("--port".to_string());
    args.push(port.to_string());

    // ===== 设置页 SGLang 详细参数（仅非空/非默认值，规则同单机）=====
    if sglang_args.context_length > 0 {
        args.extend(["--context-length".to_string(), sglang_args.context_length.to_string()]);
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
    for line in sglang_args.extra_args.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim().trim_start_matches("--");
            let v = v.trim();
            if !k.is_empty() && !v.is_empty() {
                args.push(format!("--{}", k));
                args.push(v.to_string());
            }
        }
    }

    // ===== MTP 自动启用（规则同单机）=====
    let flags = sglang_flags.as_deref().unwrap_or(&[]);
    if has_mtp_weight(model_dir) && !spec_user_defined(sglang_args, flags) {
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
    }

    // ===== 模型清单 sglang_flags（规则同单机，追加即生效）=====
    if !flags.is_empty() {
        for raw in flags {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            let (k, v) = match raw.split_once(char::is_whitespace) {
                Some((k, v)) => (k, v.trim()),
                None => (raw, ""),
            };
            let k = k.trim_start_matches("--");
            if k.is_empty() {
                continue;
            }
            args.push(format!("--{}", k));
            if !v.is_empty() {
                args.push(v.to_string());
            }
        }
    }

    // ===== 多机参数（最后追加，跨节点 TP 拓扑不可被覆盖）=====
    let n_nodes = mn.nodes.len();
    args.extend([
        "--tensor-parallel-size".to_string(),
        n_nodes.to_string(),
        "--nnodes".to_string(),
        n_nodes.to_string(),
        "--node-rank".to_string(),
        rank.to_string(),
        "--dist-init-addr".to_string(),
        format!("{}:{}", node0_ip, mn.dist_init_port),
    ]);
    if mn.nccl_port > 0 {
        args.extend(["--nccl-port".to_string(), mn.nccl_port.to_string()]);
    }
    args
}

/// 本机（rank 0）docker run 容器名（多机停止时据此识别多机模式）
fn multi_container_name(model_id: &str, rank: usize) -> String {
    format!("adm-sglang-{}-rank-{}", model_id, rank)
}

/// 停止已启动的远端节点容器（启动失败回滚 / 停止模型共用）
async fn stop_remote_containers(
    app: &tauri::AppHandle,
    mn: &MultiNodeArgs,
    key: Option<&str>,
    model_id: &str,
    ranks_start: usize,
) {
    for (i, node) in mn.nodes.iter().enumerate().skip(ranks_start) {
        if node.is_self {
            continue;
        }
        let container = multi_container_name(model_id, i);
        let script = crate::common::ssh::stop_container_script(&container);
        match crate::common::ssh::ssh_run(
            &node.ip,
            &node.ssh_user,
            node.ssh_port,
            key,
            &script,
            std::time::Duration::from_secs(20),
        )
        .await
        {
            Ok(_) => {
                crate::common::utils::logger::write_log("INFO", "MODEL", &format!("[{}] 已停止远端节点 {} 容器 {}", model_id, node.ip, container));
            }
            Err(e) => {
                crate::common::utils::logger::write_log("WARN", "MODEL", &format!("[{}] 停止远端节点 {} 失败: {}（可手动执行 docker rm -f {}）", model_id, node.ip, e, container));
                let _ = app.emit("model-log", serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[WARN] 停止远端 {} 失败: {}；可手动执行 docker rm -f {}", node.ip, e, container),
                    "source": "stderr",
                }));
            }
        }
    }
}

/// 多机启动：节点 0（本机）docker run + 远端 SSH nohup docker run，
/// 全部就绪（本机 Uvicorn running on / 远端容器 Up）前任一失败即回滚。
async fn start_multi_node(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
    model_id: &str,
    model_dir: &std::path::Path,
    params: LaunchParams,
    sglang_version: Option<String>,
    sglang_flags: Option<Vec<String>>,
) -> Result<(), AppError> {
    let (mn, sglang_args) = load_multi_node_config(app);
    let port: u16 = params.port.unwrap_or(5678);
    validate_multi_node(&mn, port)?;
    let n_nodes = mn.nodes.len();
    let key: Option<String> = if mn.ssh_key_path.trim().is_empty() {
        None
    } else {
        Some(mn.ssh_key_path.trim().to_string())
    };
    let key_ref = key.as_deref();

    // 镜像 / 共享内存：设置页优先，缺省用机型默认（规则同单机）
    let default_image = "lmsysorg/sglang:v0.5.17";
    let default_shm = "64g";
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
    let shm_size = if sglang_args.shm_size.is_empty() { default_shm.to_string() } else { sglang_args.shm_size.clone() };

    // 本机 Docker 预检（CLI / daemon / GPU runtime / 镜像，含自动 pull）
    let image = check_docker_env(app, model_id, &image).await?;
    // 本机端口占用检查（--network host 下端口直接 bind 宿主机）
    {
        let probe = std::net::TcpListener::bind(("0.0.0.0", port));
        if probe.is_err() {
            bail!("端口 {} 已被占用，请先关闭占用该端口的进程，或在设置页更换监听端口", port);
        }
    }
    // 引导端口占用检查
    {
        let probe = std::net::TcpListener::bind(("0.0.0.0", mn.dist_init_port));
        if probe.is_err() {
            bail!("引导端口 {} 已被占用，请在设置页更换引导端口", mn.dist_init_port);
        }
    }

    crate::common::utils::logger::write_log(
        "INFO",
        "MODEL",
        &format!("[{}] 多机模式启动：{} 个节点（TP={}），镜像 {}，节点0={}", model_id, n_nodes, n_nodes, image, mn.nodes[0].ip),
    );
    let _ = app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("[多机] 启动 {} 节点集群（TP={}，rank0={}），镜像 {}", n_nodes, n_nodes, mn.nodes[0].ip, image),
        "source": "stdout",
    }));

    // ===== 远端节点：预检 + 启动 + 快速就绪确认（任一失败回滚）=====
    let node0_ip = mn.nodes[0].ip.clone();
    for (i, node) in mn.nodes.iter().enumerate().skip(1) {
        let node_model_dir = effective_remote_model_dir(&node.model_dir, &node.ssh_user, model_id);
        // 预检：docker daemon / GPU / 本机所用镜像 / 模型目录（含 .done）
        let probe = crate::common::ssh::probe_script(&node_model_dir, &image, false);
        let (ok, out, err) = crate::common::ssh::ssh_run(
            &node.ip, &node.ssh_user, node.ssh_port, key_ref, &probe,
            std::time::Duration::from_secs(20),
        ).await.map_err(|e| {
            AppError::msg(format!("远端节点 {}（rank {}）探活失败: {}", node.ip, i, e))
        })?;
        let mut docker_ok = false;
        let mut model_ok = false;
        let mut image_ok = false;
        let mut gpu = String::new();
        for line in out.lines().chain(err.lines()) {
            if let Some(v) = line.strip_prefix("GPU:") {
                gpu = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("DOCKER:") {
                docker_ok = v.trim() != "DOCKER_ERR" && !v.trim().is_empty();
            } else if let Some(v) = line.strip_prefix("IMAGE:") {
                image_ok = v.trim() == "IMAGE_OK";
            } else if let Some(v) = line.strip_prefix("MODEL:") {
                model_ok = v.trim() == "MODEL_OK";
            }
        }
        if !ok {
            bail!("远端节点 {}（rank {}）SSH 执行失败: {}", node.ip, i, if err.is_empty() { "未知错误" } else { err.as_str() });
        }
        if !docker_ok {
            bail!("远端节点 {}（rank {}）Docker daemon 不可用", node.ip, i);
        }
        if !image_ok {
            bail!("远端节点 {}（rank {}）未下载镜像 {}（请先在远端 docker pull 或配置镜像加速）", node.ip, i, image);
        }
        if !model_ok {
            bail!("远端节点 {}（rank {}）模型目录不存在或未下载完成：{}（可先在设置页「同步模型到直连节点」自动同步）", node.ip, i, node_model_dir);
        }
        let _ = app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": format!("[多机] 节点 {}（rank {}）环境正常：GPU={} Docker={} 镜像={}", node.ip, i, gpu, "OK", "OK"),
            "source": "stdout",
        }));

        // 启动：nohup docker run 后台运行，日志落盘 /tmp/adm_sglang_<model>_rank_<i>.log
        // 自动从节点 IP 反查互连网卡名
        let remote_iface = detect_remote_iface(
            &node.ip, &node.ssh_user, node.ssh_port, key_ref, &node.ip,
        ).await;
        if let Some(ref iface_name) = remote_iface {
            let _ = app.emit("model-log", serde_json::json!({
                "model_id": model_id,
                "line": format!("[多机] 节点 {}（rank {}）互连网卡：{}", node.ip, i, iface_name),
                "source": "stdout",
            }));
        }
        let args = build_multi_node_args(
            model_id,
            &std::path::Path::new(&node_model_dir),
            &image,
            &shm_size,
            port,
            &sglang_args,
            &sglang_flags,
            &mn,
            i,
            &node0_ip,
            remote_iface.as_deref(),
        );
        let container = multi_container_name(model_id, i);
        let log_path = format!("/tmp/adm_sglang_{}_rank_{}.log", model_id, i);
        // 远端 docker 权限探测：免密 sudo 可用时 docker run 加 sudo -n（非 docker 组环境兜底）
        let (rsok, rcout, _) = crate::common::ssh::ssh_run(
            &node.ip, &node.ssh_user, node.ssh_port, key_ref,
            "sudo -n true 2>/dev/null && echo SUDO_OK || echo SUDO_NO",
            std::time::Duration::from_secs(15),
        ).await.map_err(|e| {
            AppError::msg(format!("远端节点 {}（rank {}）探活失败: {}", node.ip, i, e))
        })?;
        let use_sudo = rsok && rcout.contains("SUDO_OK");
        let script = crate::common::ssh::start_container_script(&container, &args, &log_path, use_sudo);
        let (sok, _sout, serr) = crate::common::ssh::ssh_run(
            &node.ip, &node.ssh_user, node.ssh_port, key_ref, &script,
            std::time::Duration::from_secs(30),
        ).await.map_err(|e| {
            AppError::msg(format!("远端节点 {}（rank {}）启动命令执行失败: {}", node.ip, i, e))
        })?;
        if !sok {
            let _ = stop_remote_containers(app, &mn, key_ref, model_id, 1).await;
            bail!("远端节点 {}（rank {}）docker run 发起失败: {}", node.ip, i, serr);
        }

        // 快速就绪确认：30s 内容器进入 Up（多数启动失败（镜像缺失/参数错）会立即 Exited）
        let mut up = false;
        let mut tail = String::new();
        for _ in 0..15 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let chk = format!("(sudo -n docker ps --filter name={} --format '{{{{\"n\": .Names, \"s\": .Status}}}}' 2>/dev/null || docker ps --filter name={} --format '{{{{\"n\": .Names, \"s\": .Status}}}}' 2>/dev/null)", crate::common::ssh::sh_quote(&container), crate::common::ssh::sh_quote(&container));
            if let Ok((_, out2, _)) = crate::common::ssh::ssh_run(
                &node.ip, &node.ssh_user, node.ssh_port, key_ref, &chk,
                std::time::Duration::from_secs(10),
            ).await {
                if out2.contains(&format!("\"n\":\"{}\"", container)) && out2.contains("\"s\":\"Up") {
                    up = true;
                    break;
                }
                if out2.contains("Exited") || out2.contains("Dead") {
                    if let Ok((_, t2, _)) = crate::common::ssh::ssh_run(
                        &node.ip, &node.ssh_user, node.ssh_port, key_ref,
                        &format!("tail -n 30 {}", crate::common::ssh::sh_quote(&log_path)),
                        std::time::Duration::from_secs(10),
                    ).await {
                        tail = t2;
                    }
                    break;
                }
            }
        }
        if !up {
            let _ = stop_remote_containers(app, &mn, key_ref, model_id, 1).await;
            let detail = if tail.is_empty() { "30s 内未进入运行状态".to_string() } else { tail };
            crate::common::utils::logger::write_log("ERROR", "MODEL", &format!("[{}] 远端节点 {} 容器启动失败:\n{}", model_id, node.ip, detail));
            bail!("远端节点 {}（rank {}）容器启动失败，已回滚停止已启动节点：\n{}", node.ip, i, detail);
        }
        let _ = app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": format!("[多机] 远端节点 {}（rank {}）容器已就绪", node.ip, i),
            "source": "stdout",
        }));
    }

    // ===== 本机（rank 0）=====
    let container0 = multi_container_name(model_id, 0);
    // 清理同名残留容器
    let _ = crate::common::utils::platform::docker_cmd()
        .args(["rm", "-f", &container0])
        .output();
    // 自动从本机节点 IP 反查互连网卡名
    let local_iface = detect_local_iface(&node0_ip);
    if let Some(ref iface_name) = local_iface {
        let _ = app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": format!("[多机] 本机（rank 0）互连网卡：{}", iface_name),
            "source": "stdout",
        }));
    }
    let args0 = build_multi_node_args(
        model_id, model_dir, &image, &shm_size, port, &sglang_args, &sglang_flags, &mn, 0, &node0_ip, local_iface.as_deref(),
    );

    dbg_log!("[DEBUG] sglang multi-node docker args (rank0): {:?}", args0);
    let _ = app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("[多机] 本机（rank 0）启动容器 {}", container0),
        "source": "stdout",
    }));

    let mut child = match spawn_docker_run(&args0) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("启动推理引擎容器失败: {}", e);
            let _ = app.emit("model-log", serde_json::json!({
                "model_id": model_id, "line": format!("[ERROR] {}", msg), "source": "stderr",
            }));
            // 任一失败回滚：停止已启动的远端节点容器
            stop_remote_containers(app, &mn, key_ref, model_id, 1).await;
            return Err(AppError::msg(msg));
        }
    };

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
        *container_lock = Some(container0.clone());
    }
    state.set_model_running(true);
    state.bump_model_generation();

    let _ = app.emit("model-started", serde_json::json!({ "model_id": model_id, "port": port }));

    // ===== 本机 stdout/stderr 转发 + 退出监控（规则同单机；退出时尽力停止远端）=====
    let app_clone = app.clone();
    let model_id_clone = model_id.to_string();
    let app_clone2 = app.clone();
    let model_id_clone2 = model_id.to_string();
    let container_clone0 = container0.clone();
    let mn_clone = mn.clone();
    let key_clone = key.clone();

    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};

        let stdout_handle = if let Some(stdout) = child.stdout.take() {
            let app_c = app_clone.clone();
            let mid = model_id_clone.clone();
            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    crate::common::utils::logger::write_log("INFO", "SGLang", &line);
                    app_c.emit("model-log", serde_json::json!({
                        "model_id": &mid, "line": line.clone(), "source": "stdout",
                    })).ok();
                    if line.contains("Uvicorn running on")
                        || line.contains("The server is fired up and ready to rock!")
                    {
                        app_c.emit("model-started", serde_json::json!({
                            "model_id": &mid, "port": port,
                        })).ok();
                    }
                }
            }))
        } else {
            None
        };

        let stderr_handle = if let Some(stderr) = child.stderr.take() {
            let app_c = app_clone2.clone();
            let mid = model_id_clone2.clone();
            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    crate::common::utils::logger::write_log("WARN", "SGLang", &line);
                    app_c.emit("model-log", serde_json::json!({
                        "model_id": &mid, "line": line, "source": "stderr",
                    })).ok();
                }
            }))
        } else {
            None
        };

        if let Some(h) = stdout_handle { let _ = h.join(); }
        if let Some(h) = stderr_handle { let _ = h.join(); }
        let _ = child.wait();

        // 容器退出：清理本机容器与状态，并尽力停止远端节点
        let _ = crate::common::utils::platform::docker_cmd()
            .args(["rm", "-f", &container_clone0])
            .output();
        // 远端停止：同步版 SSH（每台最多 3s，尽力而为，失败仅写日志）
        let key_ref = key_clone.as_deref();
        for (i, node) in mn_clone.nodes.iter().enumerate().skip(1) {
            if node.is_self { continue; }
            let c = multi_container_name(&model_id_clone2, i);
            let script = crate::common::ssh::stop_container_script(&c);
            let _ = crate::common::ssh::ssh_run_blocking(
                &node.ip, &node.ssh_user, node.ssh_port, key_ref, &script,
                std::time::Duration::from_secs(3),
            );
        }
        app_clone2.emit("model-log", serde_json::json!({
            "model_id": &model_id_clone2,
            "line": "[多机] 节点 0 容器已退出，已尝试停止远端节点容器",
            "source": "stdout",
        })).ok();

        {
            let state = app_clone2.state::<AppState>();
            *state.running_process.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *state.running_model_id.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *state.running_port.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *state.running_container.lock().unwrap_or_else(|e| e.into_inner()) = None;
            state.set_model_running(false);
        }
        app_clone2.emit("model-stopped", serde_json::json!({ "model_id": &model_id_clone2 })).ok();
    });

    // ===== 远端运行期监控：容器异常退出时转发日志告警（不自动级联停止）=====
    {
        let app_c = app.clone();
        let mid = model_id.to_string();
        let mn_c = mn.clone();
        let key_c = key.clone();
        let container0_c = container0.clone();
        std::thread::spawn(move || {
            let key_ref = key_c.as_deref();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(10));
                // 模型已停止（stop_model / 本机容器退出已统一清理远端）→ 静默退出，避免停止过程中的误报
                let still_running = app_c
                    .state::<AppState>()
                    .running_container
                    .lock()
                    .ok()
                    .and_then(|l| l.clone())
                    .map(|c| c == container0_c)
                    .unwrap_or(false);
                if !still_running {
                    break;
                }
                let mut all_down = true;
                let mut checked = false;
                for (i, node) in mn_c.nodes.iter().enumerate().skip(1) {
                    if node.is_self { continue; }
                    let c = multi_container_name(&mid, i);
                    let chk = format!("(sudo -n docker ps --filter name={} --format '{{{{\"n\": .Names, \"s\": .Status}}}}' 2>/dev/null || docker ps --filter name={} --format '{{{{\"n\": .Names, \"s\": .Status}}}}' 2>/dev/null)", crate::common::ssh::sh_quote(&c), crate::common::ssh::sh_quote(&c));
                    let status = crate::common::ssh::ssh_run_blocking(
                        &node.ip, &node.ssh_user, node.ssh_port, key_ref, &chk,
                        std::time::Duration::from_secs(3),
                    );
                    match status {
                        Ok((_, out, _)) if out.is_empty() => {
                            // 容器不存在（可能已手动停止）：视为 down
                            checked = true;
                        }
                        Ok((_, out, _)) if out.contains("Exited") || out.contains("Dead") => {
                            checked = true;
                            let log_path = format!("/tmp/adm_sglang_{}_rank_{}.log", mid, i);
                            let tail = crate::common::ssh::ssh_run_blocking(
                                &node.ip, &node.ssh_user, node.ssh_port, key_ref,
                                &format!("tail -n 30 {}", crate::common::ssh::sh_quote(&log_path)),
                                std::time::Duration::from_secs(3),
                            );
                            let detail = tail.map(|(_, t, _)| t).unwrap_or_default();
                            crate::common::utils::logger::write_log("ERROR", "MODEL", &format!("[{}] 远端节点 {}（rank {}）容器异常退出:\n{}", mid, node.ip, i, detail));
                            app_c.emit("model-log", serde_json::json!({
                                "model_id": &mid,
                                "line": format!("[ERROR] 远端节点 {}（rank {}）容器异常退出，请查看模型日志并停止模型：\n{}", node.ip, i, detail),
                                "source": "stderr",
                            })).ok();
                        }
                        Ok((_, out, _)) if out.contains("\"s\":\"Up") => {
                            all_down = false;
                            checked = true;
                        }
                        _ => {
                            // SSH 失败等不确定状态：不判定
                            all_down = false;
                        }
                    }
                }
                // 所有远端节点均已确认消失/退出时停止监控（节点 0 退出时另有统一清理）
                if checked && all_down { break; }
            }
        });
    }

    Ok(())
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
    sglang_flags: Option<Vec<String>>,
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
    // 镜像优先级：模型配置 sglang_version（完整镜像名）> 设置页镜像 > 机型默认。
    // sglang_version 直接就是完整镜像名（如 lmsysorg/sglang:dev-cu13-qwen38-27b-dflash2），原样使用不再拼接
    let mut image = if sglang_args.image.is_empty() { default_image.to_string() } else { sglang_args.image.clone() };
    if let Some(ver) = sglang_version.as_deref().map(str::trim) {
        if !ver.is_empty() {
            image = ver.to_string();
        }
    }
    crate::common::utils::logger::write_log("INFO", "DOCKER", &format!("[{}] 使用镜像 {}（优先级：模型 sglang_version > 设置页 > 默认）", model_id, image));
    app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("使用镜像 {}（模型 sglang_version / 设置页 / 默认）", image),
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
    }) || sglang_flags.as_deref().unwrap_or(&[]).iter().any(|f| f.contains("speculative-algorithm"));
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

    // ===== 模型配置 SGLang 参数（sglang_flags）=====
    // 官方 cookbook 推荐值，最后追加，优先级最高：同名参数可覆盖设置页配置与默认值
    //（SGLang 各参数为 argparse 后值生效）。每条格式 "--key value" 或 "--flag"。
    if let Some(flags) = sglang_flags.as_deref().filter(|f| !f.is_empty()) {
        let mut applied = Vec::new();
        for raw in flags {
            let raw = raw.trim();
            if raw.is_empty() { continue; }
            let (k, v) = match raw.split_once(char::is_whitespace) {
                Some((k, v)) => (k, v.trim()),
                None => (raw, ""),
            };
            let k = k.trim_start_matches("--");
            if k.is_empty() { continue; }
            args.push(format!("--{}", k));
            applied.push(format!("--{}", k));
            if !v.is_empty() {
                args.push(v.to_string());
                applied.push(v.to_string());
            }
        }
        if !applied.is_empty() {
            app.emit(
                "model-log",
                serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[模型配置] 已应用模型清单 sglang_flags（优先级最高）：{}", applied.join(" ")),
                    "source": "stdout",
                }),
            )
            .ok();
            crate::common::utils::logger::write_log("INFO", "MODEL", &format!("[{}] 应用模型清单 sglang_flags: {}", model_id, applied.join(" ")));
        }
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
    sglang_flags: Option<Vec<String>>,
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
        // 多机互联：设置页启用且节点数 >= 2 时走集群启动（单机路径不变）
        let (mn, _) = load_multi_node_config(&app);
        if mn.enabled && mn.nodes.len() >= 2 {
            return start_multi_node(&app, &state, &model_id, &model_dir, params, sglang_version, sglang_flags).await;
        }
        return start_sglang_docker(&app, &state, &model_id, &model_dir, params, device, sglang_version, sglang_flags).await;
    }

    // 仅支持 SGLang Docker 部署（safetensors 目录模型）
    Err(AppError::msg("当前仅支持推理引擎（safetensors 目录）模型，请下载新版模型后重试"))
}

#[tauri::command]
pub async fn stop_model(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    let pid = {
        let pid_lock = state.running_process.lock().map_err(|e| e.to_string())?;
        pid_lock.ok_or("没有正在运行的模型")?
    };
    // SGLang Docker 模式：先优雅停止容器，再兜底杀进程
    let container = state.running_container.lock().map_err(|e| e.to_string())?.clone();
    if let Some(container_name) = container {
        // 多机模式（容器名 adm-sglang-<model>-rank-0）：先 SSH 停止所有远端节点容器
        if container_name.ends_with("-rank-0") {
            let model_id = state
                .running_model_id
                .lock()
                .map_err(|e| e.to_string())?
                .clone()
                .unwrap_or_default();
            if !model_id.is_empty() {
                let (mn, _) = load_multi_node_config(&app);
                let key: Option<String> = if mn.ssh_key_path.trim().is_empty() {
                    None
                } else {
                    Some(mn.ssh_key_path.trim().to_string())
                };
                stop_remote_containers(&app, &mn, key.as_deref(), &model_id, 1).await;
            }
        }
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
