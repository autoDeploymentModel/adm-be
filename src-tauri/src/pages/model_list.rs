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

/// 下载实时速度跟踪器：0.5s 窗口内累计字节求平均，返回 bytes/s。
/// 用 `Mutex<SpeedTracker>` 包装，可在 Fn 闭包内做可变状态且跨 await 保持 Send。
#[derive(Clone, Copy)]
struct SpeedTracker {
    last_bytes: u64,
    window_start: std::time::Instant,
    window_bytes: u64,
    speed: u64,
}

impl SpeedTracker {
    /// `existing` = 续传前 .part 已有字节数，避免首窗把已有字节计入速度导致虚高
    fn from_existing(existing: u64) -> Self {
        Self {
            last_bytes: existing,
            window_start: std::time::Instant::now(),
            window_bytes: 0,
            speed: 0,
        }
    }
    /// 每收到一次进度回调调用；窗口不满 0.5s 时返回上一次的 speed（避免单 chunk 抖动）
    fn update(&mut self, downloaded: u64) -> u64 {
        let now = std::time::Instant::now();
        self.window_bytes += downloaded.saturating_sub(self.last_bytes);
        self.last_bytes = downloaded;
        let elapsed = now.duration_since(self.window_start).as_secs_f64();
        if elapsed >= 0.5 {
            self.speed = if elapsed > 0.0 {
                (self.window_bytes as f64 / elapsed) as u64
            } else {
                0
            };
            self.window_start = now;
            self.window_bytes = 0;
        }
        self.speed
    }
}

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
    model_files: Option<Vec<String>>,
    vllm_image: Option<String>,
) -> Result<(), AppError> {
    {
        let state = app.state::<AppState>();
        let map = state.downloading_progress.lock().map_err(|e| e.to_string())?;
        if map.contains_key(&model_id) {
            bail!("该模型正在下载中，请勿重复点击");
        }
    }

    // 下载清单校验提前到取消标志插入之前：清单缺失直接失败，
    // 不在 download_cancel 表中留下无下载对应的死条目
    if model_files.as_ref().map(|f| f.is_empty()).unwrap_or(true) {
        bail!(
            "模型 {} 缺少下载文件清单（model_download_files 必填），请检查远程 model.json 配置",
            model_id
        );
    }

    // 取消标志：前端 cancel_download 置位后，下载循环感知到即停止（保留 .part 续传）
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    let cancel_flag = Arc::new(AtomicBool::new(false));
    app.state::<AppState>()
        .download_cancel
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(model_id.clone(), cancel_flag.clone());

    let data_dir = config::get_data_dir(Some(&app))?;
    let model_dir = data_dir.join("models").join(&model_id);
    std::fs::create_dir_all(&model_dir).map_err(|e| format!("创建模型目录失败: {}", e))?;

    // ===== HF 仓库多文件目录下载（safetensors 模型，唯一支持的格式） =====
    if let Some(files) = model_files {
        {
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
                    if let Ok(mut map) = self.h.state::<AppState>().download_cancel.lock() {
                        map.remove(&self.id);
                    }
                }
            }
            let _guard = CleanupGuard2 { h: app.clone(), id: model_id.clone() };

            // 下载客户端：开启代理 → 挂代理 + Direct（源链接直下）；否则 hf-mirror 优先
            let http = crate::common::utils::proxy::build_download_http(&app, None).await?;

            for (idx, url) in files.iter().enumerate() {
                // 镜像策略见 download_with_resume 的 MirrorPolicy（代理开启时直接源链接）
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
                // 实时下载速度（bytes/s），0.5s 窗口平滑
                let part_existing = std::fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);
                let speed_tracker = std::sync::Mutex::new(SpeedTracker::from_existing(part_existing));
                match download_with_resume(
                    &http.client, &url, &final_path, &part_path, http.mirror_policy, Some(cancel_flag.clone()),
                    |progress, downloaded, _total| {
                        let speed = {
                            let mut st = speed_tracker.lock().unwrap_or_else(|e| e.into_inner());
                            st.update(downloaded)
                        };
                        // 总进度 = 已完成文件 + 当前文件进度折算
                        let overall = ((idx_f as f32 + progress as f32 / 100.0) * 100.0 / total_f as f32) as u8;
                        app_clone.emit(
                            "download-progress",
                            serde_json::json!({
                                "model_id": &mid,
                                "progress": overall,
                                "speed": speed,
                                "file": &fname,
                                "type": "model",
                            }),
                        ).ok();
                        if let Ok(mut map) = app_clone.state::<AppState>().downloading_progress.lock() {
                            map.insert(mid.clone(), overall);
                        }
                    },
                ).await {
                    Ok(()) => {}
                    Err(e) if crate::common::utils::download::is_cancelled(&e) => {
                        app.emit(
                            "download-cancelled",
                            serde_json::json!({ "model_id": &model_id, "type": "model" }),
                        ).ok();
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                }
                // 单个文件完成不单独发 download-complete（前端收到会清空 downloading
                // 状态导致按钮闪变），进度折算继续由 download-progress 驱动，
                // 全部完成时集中发一次带 all=true 的完成事件。
            }

            // 全部文件下载完成：写 .done 标记（scan_local_models 排除）
            std::fs::write(model_dir.join(".done"), "").ok();
            // 模型文件下完 → 立刻发完成事件（前端立即显示「已下载」+ 启动按钮可点）。
            app.emit(
                "download-complete",
                serde_json::json!({ "model_id": &model_id, "type": "model", "all": true }),
            ).ok();
            // 镜像拉取阶段记入 downloading_phase（CleanupGuard2 函数返回时清理）：
            // start_model 据此拒绝并发启动，避免「已下载后点击启动」与下载阶段的后台
            // 拉镜并发执行 docker pull（同一镜像双 pull）。
            if let Ok(mut map) = app.state::<AppState>().downloading_phase.lock() {
                map.insert(model_id.clone(), "<pull-image>".to_string());
            }
            // ===== 镜像策略：远程 model.json 的 vllm_image 字段唯一指定 =====
            // 镜像名必须由后端重新拉取远程清单解析（与启动流程一致），不能依赖前端
            // 卡片透传——远程配置更新后旧卡片透传值为空，会导致镜像被静默跳过，
            // 最终启动时才报「镜像尚未下载」。下载前先 docker 检查，缺失才拉。
            let resolved_image = resolve_vllm_image(&model_id, vllm_image.as_deref()).await;
            if let Some(image) = resolved_image {
                pull_image_if_configured(&app, &model_id, Some(&image)).await;
            } else {
                // 无法解析镜像名（远程清单拉取失败且无前端兜底，或清单缺 vllm_image 字段）：明确报错，不静默
                let msg = format!(
                    "[ERROR] 模型 {} 无法解析 vllm_image 配置（远程 model.json 必填字段，或远程清单拉取失败），镜像未拉取；请检查网络后重新点击「下载」触发",
                    model_id
                );
                crate::common::utils::logger::write_log("ERROR", "DOWNLOAD", &msg);
                app.emit(
                    "model-log",
                    serde_json::json!({
                        "model_id": &model_id,
                        "line": msg,
                        "source": "stderr",
                    }),
                ).ok();
                app.emit(
                    "download-complete",
                    serde_json::json!({
                        "model_id": &model_id,
                        "type": "image-pull-failed",
                        "image": "",
                        "error": "无法解析 vllm_image 配置（远程 model.json 必填字段，或远程清单拉取失败）",
                    }),
                ).ok();
            }
            return Ok(());
        }
    }

    // model_url 旧格式单文件下载已废弃：新格式模型必须带 model_download_files
    bail!(
        "模型 {} 缺少下载文件清单（model_download_files 必填），请检查远程 model.json 配置",
        model_id
    );
}

/// 从远程 model.json 解析指定模型的权威 `vllm_image`（与启动流程一致，镜像由远程唯一指定）。
/// 远程拉取失败时回退到前端透传的 fallback；两者都没有返回 None。
async fn resolve_vllm_image(model_id: &str, fallback: Option<&str>) -> Option<String> {
    let from_fallback = || {
        fallback
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };
    match fetch_model_list().await {
        Ok(list) => list
            .iter()
            .find(|m| m.model_id == model_id)
            .map(|m| m.vllm_image.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(from_fallback),
        Err(_) => from_fallback(),
    }
}

/// 模型下载完成后拉取 vLLM 镜像（如果模型清单指定了 vllm_image）：
/// - 配置缺字段（None / 空串）→ 直接跳过（启动时会被拒绝）
/// - 镜像已存在 → 跳过拉取，秒级返回
/// - 拉取成功 → 后续 start_vllm_docker 可直接 docker run
/// - 拉取失败 → 写日志 + 发 model-log + toast，**不报错返回**（模型文件已落盘，
///   用户可手动 docker pull 或重新点击下载触发重拉）
async fn pull_image_if_configured(app: &tauri::AppHandle, model_id: &str, image: Option<&str>) {
    let image = match image.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => v.to_string(),
        None => return,
    };
    match pull_docker_image(app, model_id, &image).await {
        Ok(real_image) => {
            crate::common::utils::logger::write_log("INFO", "DOCKER", &format!("[{}] 镜像拉取完成: {}", model_id, real_image));
            let _ = app.emit(
                "model-log",
                serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[Docker] 镜像 {} 拉取完成，已就绪", real_image),
                    "source": "stdout",
                }),
            );
        }
        Err(e) => {
            let msg = format!("镜像 {} 拉取失败（启动前需补拉）：{}", image, e);
            crate::common::utils::logger::write_log("ERROR", "DOCKER", &format!("[{}] {}", model_id, msg));
            let _ = app.emit(
                "model-log",
                serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[ERROR] {}", msg),
                    "source": "stderr",
                }),
            );
            let _ = app.emit(
                "download-complete",
                serde_json::json!({
                    "model_id": model_id,
                    "type": "image-pull-failed",
                    "image": &image,
                    "error": e.to_string(),
                }),
            );
        }
    }
}

/// Docker 环境预检：CLI 存在 → daemon 运行 → NVIDIA runtime 可用。
/// **镜像缺失时当场拉取**（pull 进度经 model-pull-progress 事件驱动前端「拉取镜像 X%」），
/// 拉取失败才报错返回。任一环节失败返回错误原因，前端以 toast / model-log 展示。
async fn check_docker_env(
    app: &tauri::AppHandle,
    model_id: &str,
    image: &str,
) -> Result<String, AppError> {
    docker_preflight(app, model_id).await?;

    // 镜像存在 → 直接用；缺失 → 当场拉取（下载失败/目录拷入/配置晚更新等场景兜底），
    // 避免「启动失败：镜像尚未下载，请回列表重新点击下载」的断链式引导
    pull_docker_image(app, model_id, image).await
}

/// 拉取镜像（仅由 `download_model` 在模型文件下完后调用）：
/// CLI/daemon/GPU 预检 → 镜像存在则跳过 → 否则 `docker pull`（多源回退/超时见 pull_image）。
/// 成功返回实际可用镜像名（可能是镜像源前缀版本），拉取失败返回错误。
async fn pull_docker_image(
    app: &tauri::AppHandle,
    model_id: &str,
    image: &str,
) -> Result<String, AppError> {
    docker_preflight(app, model_id).await?;

    if docker_image_exists(app, model_id, image).await? {
        crate::common::utils::logger::write_log("INFO", "DOCKER", &format!("[{}] 镜像 {} 已存在，跳过拉取", model_id, image));
        app.emit(
            "model-log",
            serde_json::json!({
                "model_id": model_id,
                "line": format!("[Docker] 镜像 {} 已存在本地，跳过拉取", image),
                "source": "stdout",
            }),
        )
        .ok();
        return Ok(image.to_string());
    }

    // 镜像可能已拉取但以 <none>:<none>（untagged）落盘：先尝试直接修正 tag，
    // 免去重复拉取（镜像加速器 digest 不一致场景下重新 pull 也仍会 untagged）。
    // ensure_image_tag 内部带 RepoDigests 归属校验，只对与目标 repo 一致的
    // untagged 镜像打 tag，不会误伤无关的历史 dangling 镜像。
    if ensure_image_tag(app, model_id, image, None).await.is_ok() {
        return Ok(image.to_string());
    }

    match pull_image(app, model_id, image, PULL_TIMEOUT).await {
        Ok(true) => Ok(image.to_string()),
        Ok(false) => Err(AppError::msg(format!(
            "镜像 {} 拉取失败（无进度输出超时 {} 分钟）；请检查网络或镜像地址后手动执行 docker pull {}；国内网络可改用镜像加速：设置页「Docker 镜像配置」写入加速器地址（如 https://docker.1ms.run）并重启 Docker；或设置页「代理」填写本地代理并点「保存并重启 Docker」",
            image,
            PULL_TIMEOUT.as_secs() / 60,
            image
        ))),
        Err(first_err) => {
            // 闲置超时（连续无输出）常见于镜像源瞬时抽风 / 网络抖动 / 首次连接慢；
            // 已下载层会复用，自动重试一次后再失败才报错，避免来回手动重试。
            crate::common::utils::logger::write_log(
                "WARN",
                "DOCKER",
                &format!("[{}] 镜像 {} 拉取中止（{}），等待 8 秒后自动重试（第 2/2 次）", model_id, image, first_err),
            );
            let _ = app.emit(
                "model-log",
                serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[docker pull] 镜像 {} 拉取中止（{}），8 秒后自动重试（第 2/2 次）...", image, first_err),
                    "source": "stderr",
                }),
            );
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            match pull_image(app, model_id, image, PULL_TIMEOUT).await {
                Ok(true) => Ok(image.to_string()),
                Ok(false) => Err(AppError::msg(format!(
                    "镜像 {} 拉取失败（自动重试 1 次后仍无进度输出，累计耗时超 {} 分钟）；请检查网络或镜像地址：国内网络可改用镜像加速（设置页「Docker 镜像配置」）或设置页「代理」后重试；也可手动执行 docker pull {} 观察报错",
                    image,
                    (PULL_TIMEOUT.as_secs() * 2) / 60,
                    image
                ))),
                Err(second_err) => Err(AppError::msg(format!(
                    "镜像 {} 拉取中止（自动重试一次后仍失败）: {}；请检查网络（可尝试设置页「Docker 镜像配置」调整加速器或「代理」）或手动执行 docker pull {}",
                    image, second_err, image
                ))),
            }
        }
    }
}

/// CLI/daemon/GPU runtime 预检；输出镜像存在性检查外的环境信息
async fn docker_preflight(app: &tauri::AppHandle, model_id: &str) -> Result<(), AppError> {
    let log = |line: String| {
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
        if stderr.contains("permission denied") || stderr.contains("access denied") {
            return Err(AppError::msg("DOCKER_PERMISSION_DENIED".to_string()));
        }
        return Err(AppError::msg(
            "Docker daemon 未运行或不可访问，请先启动 Docker 服务（systemd: sudo systemctl start docker；桌面版: 打开 Docker Desktop）".to_string(),
        ));
    }
    let info_text = String::from_utf8_lossy(&info.stdout).to_string();
    let has_nvidia_runtime = info_text.contains("nvidia");
    let has_cdi = info_text.contains("CDI");
    log(format!(
        "[Docker] daemon 运行正常; NVIDIA runtime: {}; CDI: {}",
        if has_nvidia_runtime { "可用" } else { "未配置" },
        if has_cdi { "可用" } else { "未配置" }
    ));
    Ok(())
}

/// 检查本地是否存在指定镜像（含国内镜像源前缀探测）；存在返回 true
async fn docker_image_exists(
    app: &tauri::AppHandle,
    model_id: &str,
    image: &str,
) -> Result<bool, AppError> {
    let log = |line: String| {
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
    let inspect = crate::common::utils::platform::docker_cmd_tokio()
        .args(["image", "inspect", image])
        .output()
        .await
        .map_err(|e| AppError::msg(format!("docker image inspect 执行失败: {}", e)))?;
    if inspect.status.success() {
        log(format!("[Docker] 镜像 {} 已存在", image));
        return Ok(true);
    }
    Ok(false)
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
    // 客户端侧代理 env 兜底（rootless / podman-docker 等客户端直连场景）；
    // 标准 dockerd 拉取由 daemon 完成，需设置页「代理」→「保存并重启 Docker」
    // 把代理写入 daemon.json proxies 才真正生效。
    let mut docker_cmd = crate::common::utils::platform::docker_cmd_tokio();
    let proxy_url = crate::common::utils::proxy::proxy_url(app).await;
    if !proxy_url.is_empty() {
        crate::common::utils::logger::write_log(
            "INFO",
            "DOCKER",
            &format!("[{}] docker pull 注入代理: {}", model_id, proxy_url),
        );
        let no_proxy = "localhost,127.0.0.0/8,::1";
        docker_cmd
            .env("HTTP_PROXY", &proxy_url)
            .env("HTTPS_PROXY", &proxy_url)
            .env("ALL_PROXY", &proxy_url)
            .env("http_proxy", &proxy_url)
            .env("https_proxy", &proxy_url)
            .env("all_proxy", &proxy_url)
            .env("NO_PROXY", no_proxy)
            .env("no_proxy", no_proxy);
    }
    let mut child = docker_cmd
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
    // pull 输出的 manifest digest（`Digest: sha256:...` 行）：成功后用于把
    // untagged 镜像精确 tag 回原名（比按创建时间找 dangling 更可靠）
    let pull_digest: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

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
        let dig2 = pull_digest.clone();
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

                    // 捕获 manifest digest（如 `Digest: sha256:abcd...`），供随后 tag 修正
                    if let Some(d) = trimmed.strip_prefix("Digest:") {
                        let d = d.trim().to_string();
                        if d.starts_with("sha256:") && d.len() > "sha256:".len() {
                            *dig2.lock().unwrap_or_else(|e| e.into_inner()) = Some(d);
                        }
                    }

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
                            // 层完成 ≠ pull 完成（进程还需 digest 校验/收尾），估算封顶 99%，
                            // 仅当 pull 进程成功退出时才发 100%，避免「100% 实际仍在 pull」。
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

    // 闲置超时熔断：仅当连续 timeout 无任何输出（进度停滞/卡死）才终止；
    // 进度在动则永远等待
    tokio::select! {
        status = child.wait() => {
            let status = status.map_err(|e| AppError::msg(format!("docker pull 等待失败: {}", e)))?;
            let ok = status.success();
            emit_progress(if ok { 100 } else { 0 });
            if ok {
                // 镜像加速源/registry 交互可能导致镜像以 <none>:<none>（untagged）
                // 形式落盘——tag 丢失后 docker image inspect <image> 永远失败，
                // 会造成每次启动都误判「镜像缺失」重新拉取。这里兜底把 tag 修正
                // 为与 vllm_image 一致的名称，保证后续 inspect / docker run 可用。
                let digest = pull_digest.lock().unwrap_or_else(|e| e.into_inner()).clone();
                ensure_image_tag(app, model_id, image, digest.as_deref()).await?;
            }
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

/// pull 成功后校验镜像 tag：`docker image inspect <image>` 失败（镜像以
/// `<none>:<none>` untagged 形式落盘）时依次尝试：
/// 1. 用 pull 输出捕获的 manifest digest 精确 tag（`docker tag <repo>@sha256:<hex> <image>`）；
/// 2. 从 dangling 镜像中按创建时间取最新、且 RepoDigests 归属与目标 repo 一致的
///    重新 `docker tag`（校验归属，避免把无关历史 dangling 镜像误打 tag）；
/// 修正后再 inspect 确认。
async fn ensure_image_tag(
    app: &tauri::AppHandle,
    model_id: &str,
    image: &str,
    digest: Option<&str>,
) -> Result<(), AppError> {
    let log = |line: String| {
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

    // 1. tag 已就位（正常场景）→ 直接返回
    if docker_image_exists(app, model_id, image).await? {
        return Ok(());
    }

    // 2. 精确修正：pull 输出的 manifest digest（剥离 tag 用 <repo>@sha256:<hex> 引用）
    if let Some(d) = digest {
        let repo = split_image_repo_tag(image).0;
        let dig_ref = format!("{}@{}", repo, d);
        let tag = crate::common::utils::platform::docker_cmd_tokio()
            .args(["tag", &dig_ref, image])
            .output()
            .await;
        if let Ok(out) = tag {
            if out.status.success() {
                if docker_image_exists(app, model_id, image).await? {
                    log(format!("[Docker] 镜像 {} 拉取后为 untagged，已按 digest 修正 tag", image));
                    return Ok(());
                }
            }
        }
    }

    // 3. 兜底：dangling（untagged）中创建时间最新、RepoDigests 归属匹配的镜像
    let out = crate::common::utils::platform::docker_cmd_tokio()
        .args(["images", "--filter", "dangling=true", "--format", "{{.CreatedAt}}|{{.ID}}"])
        .output()
        .await
        .map_err(|e| AppError::msg(format!("docker images 执行失败: {}", e)))?;
    if !out.status.success() {
        return Err(AppError::msg(format!(
            "镜像 {} 拉取完成但 tag 校验失败（docker images 查询失败: {}）",
            image,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let mut best: Option<(String, String)> = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some((ts, id)) = line.split_once('|') {
            let ts = ts.trim().to_string();
            let id = id.trim().to_string();
            if best.as_ref().map_or(true, |(t, _)| ts > *t) {
                best = Some((ts, id));
            }
        }
    }
    let (_, id) = best.ok_or_else(|| {
        AppError::msg(format!(
            "镜像 {} 拉取后未在本地找到镜像（含 untagged 兜底），请手动执行 docker pull {}",
            image, image
        ))
    })?;

    // 归属校验：untagged 镜像的 RepoDigests 必须包含目标 repo（如 `ghcr.io/xxx/yyy@sha256:`），
    // 否则是无关的历史 dangling 镜像，拒绝打 tag（宁缺毋滥）。
    let repo = split_image_repo_tag(image).0;
    let inspect = crate::common::utils::platform::docker_cmd_tokio()
        .args(["image", "inspect", "--format", "{{.RepoDigests}}", &id])
        .output()
        .await
        .map_err(|e| AppError::msg(format!("docker image inspect 执行失败: {}", e)))?;
    if !inspect.status.success() {
        return Err(AppError::msg(format!(
            "镜像 {} 拉取后 tag 校验失败（无法检查 dangling 镜像 {} 归属）",
            image, id
        )));
    }
    let repo_digests = String::from_utf8_lossy(&inspect.stdout);
    if !repo_digests.contains(&format!("{}@sha256:", repo)) {
        return Err(AppError::msg(format!(
            "镜像 {} 拉取后 tag 校验失败：本地最新 dangling 镜像 {} 不属于 {}（请手动 docker pull {} 或将 docker rmi 清理无关镜像后重试）",
            image, id, repo, image
        )));
    }

    let tag = crate::common::utils::platform::docker_cmd_tokio()
        .args(["tag", &id, image])
        .output()
        .await
        .map_err(|e| AppError::msg(format!("docker tag 执行失败: {}", e)))?;
    if !tag.status.success() {
        return Err(AppError::msg(format!(
            "镜像 {} untagged 兜底失败（docker tag {} {}: {}）",
            image,
            id,
            image,
            String::from_utf8_lossy(&tag.stderr).trim()
        )));
    }
    // 3. 最终确认
    if docker_image_exists(app, model_id, image).await? {
        log(format!("[Docker] 镜像 {} 拉取后为 untagged，已从 {} 修正 tag 为 {}", image, id, image));
        return Ok(());
    }
    Err(AppError::msg(format!(
        "镜像 {} 拉取后 tag 校验仍未通过，请手动执行 docker pull {}",
        image, image
    )))
}

/// 拆分镜像名：`<repo>` 与可选 `<tag>`（冒号右侧不含 `/` 才视为 tag，兼容带端口 registry）。
fn split_image_repo_tag(image: &str) -> (&str, Option<&str>) {
    match image.rsplit_once(':') {
        Some((repo, tag)) if !tag.contains('/') => (repo, Some(tag)),
        _ => (image, None),
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
// 原则：单机路径（start_vllm_docker）零改动；仅当设置页 multi_node_args.enabled 且节点数 >= 2 时走本分支。

/// 从 config.json 读取多机配置（不存在/解析失败回默认值）

/// 从 config.json 读取多机配置（不存在/解析失败回默认值）
fn load_multi_node_config(app: &tauri::AppHandle) -> (MultiNodeArgs, VllmArgs) {
    let mut mn = MultiNodeArgs::default();
    let mut va = VllmArgs::default();
    if let Ok(settings_path) = config::get_data_dir(Some(app)).map(|d| d.join("config.json")) {
        if let Ok(json) = std::fs::read_to_string(settings_path) {
            if let Ok(parsed) = serde_json::from_str::<Settings>(&json) {
                mn = parsed.multi_node_args;
                va = parsed.vllm_args;
            }
        }
    }
    (mn, va)
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
        bail!("多机 master 端口（--master-port）不能为 0");
    }
    if mn.dist_init_port == port {
        bail!("多机 master 端口 {} 与模型服务端口冲突，请在设置页修改 master 端口", port);
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

/// 共享 docker run 前缀（多机 head/worker 都用）：
/// `--name --gpus --shm-size --cap-add --ulimit --ipc host --network host` +
/// 可选 `--device /dev/infiniband` + 可选 fork `get_env_flags` 全套网卡 env（VLLM_HOST_IP /
/// MN_IF_NAME / UCX_NET_DEVICES / NCCL_SOCKET_IFNAME / OMPI_MCA_btl_tcp_if_include /
/// GLOO_SOCKET_IFNAME / TP_SOCKET_IFNAME / NCCL_IB_HCA / NCCL_IB_DISABLE）+ 
/// `multi_node_args.extra_env` 每行 `-e KEY=VALUE` + `vllm_env`（模型清单）每行 `-e KEY=VALUE` 
/// + `NCCL_DEBUG=INFO` 默认值。模型清单 env 在 extra_env 之后注入（docker 同名 `-e` 后者生效），
/// 与 vllm_flags「模型清单优先级最高」的语义一致。
/// `node_ip`：该容器所在节点自身的互联 IP（VLLM_HOST_IP 用，避免 vllm 取 WiFi 等错误地址）。
fn build_common_docker_prefix(
    container_name: &str,
    shm_size: &str,
    node_ip: &str,
    iface: Option<&str>,
    ib_iface: Option<&str>,
    has_infiniband: bool,
    mn_extra_env: &str,
    model_env: &[String],
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "run".to_string(),
        "-e".to_string(),
        "PYTHONWARNINGS=ignore::FutureWarning".to_string(),
        "--name".to_string(),
        container_name.to_string(),
        "--gpus".to_string(),
        "all".to_string(),
        "--shm-size".to_string(),
        shm_size.to_string(),
        "--cap-add".to_string(),
        "SYS_NICE".to_string(),
        "--ulimit".to_string(),
        "stack=67108864".to_string(),
        "--ulimit".to_string(),
        "memlock=-1:-1".to_string(),
        "--cap-add".to_string(),
        "IPC_LOCK".to_string(),
        "--ipc".to_string(),
        "host".to_string(),
        "--network".to_string(),
        "host".to_string(),
    ];
    // DGX Spark 有 IB 网卡；其他机型无 IB 自动跳过
    if has_infiniband {
        args.push("--device".to_string());
        args.push("/dev/infiniband".to_string());
        // 对齐 fork get_env_flags：IB 存在时注入 NCCL_IB_HCA 并显式 NCCL_IB_DISABLE=0
        // （不指定 HCA 时 NCCL 可能自检到错误设备卡在建连；IB 不可用时由用户 extra_env 覆盖 IB_DISABLE=1）
        if let Some(ib) = ib_iface.filter(|s| !s.trim().is_empty()) {
            args.extend([
                "-e".to_string(),
                format!("NCCL_IB_HCA={}", ib.trim()),
                "-e".to_string(),
                "NCCL_IB_DISABLE=0".to_string(),
            ]);
        }
    }
    // 网卡名注入 NCCL/GLOO（多机 collective 必需）；None 时让 NCCL 自动发现
    // 对齐 fork launch-cluster.sh 的 get_env_flags，避免多网卡（WiFi/docker0）抢互联口：
    // VLLM_HOST_IP 让 vllm 用互联 IP（否则可能取 WiFi 地址导致跨节点握手走错网卡）；
    // TP_SOCKET_IFNAME/UCX_NET_DEVICES/OMPI_MCA_btl_tcp_if_include 显式约束通信接口；
    // MN_IF_NAME 为 fork 自定义变量。
    if let Some(iface_name) = iface.filter(|s| !s.trim().is_empty()) {
        let ifn = iface_name.trim();
        args.extend([
            "-e".to_string(),
            format!("VLLM_HOST_IP={}", node_ip),
            "-e".to_string(),
            format!("MN_IF_NAME={}", ifn),
            "-e".to_string(),
            format!("UCX_NET_DEVICES={}", ifn),
            "-e".to_string(),
            format!("NCCL_SOCKET_IFNAME={}", ifn),
            "-e".to_string(),
            format!("OMPI_MCA_btl_tcp_if_include={}", ifn),
            "-e".to_string(),
            format!("GLOO_SOCKET_IFNAME={}", ifn),
            "-e".to_string(),
            format!("TP_SOCKET_IFNAME={}", ifn),
        ]);
    }
    // 设置页「额外环境变量」每行 KEY=VALUE → -e KEY=VALUE（NCCL 排查如 NCCL_DEBUG=TRACE / NCCL_SOCKET_NTHREADS=1）
    let mut extra_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in mn_extra_env.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            if !k.is_empty() && !v.is_empty() {
                extra_keys.insert(k.to_string());
                args.push("-e".to_string());
                args.push(format!("{}={}", k, v));
            }
        }
    }
    // 模型清单 vllm_env（每行 KEY=VALUE → -e KEY=VALUE）：紧跟 extra_env 之后注入，
    // 同名键后者生效 → 模型清单 env 优先级最高（与 vllm_flags 语义一致，用户可在设置页 extra_env 覆盖同名键之外的变量）
    for raw in model_env {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        if let Some((k, v)) = raw.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            if !k.is_empty() && !v.is_empty() {
                extra_keys.insert(k.to_string());
                args.push("-e".to_string());
                args.push(format!("{}={}", k, v));
            }
        }
    }
    // NCCL_DEBUG 默认 INFO（extra_env / vllm_env 未显式设置时注入；与 doc §4.4 的 WARN 略有差异，便于双机排障）
    if !extra_keys.contains("NCCL_DEBUG") {
        args.push("-e".to_string());
        args.push("NCCL_DEBUG=INFO".to_string());
    }
    // DGX Spark 多机默认注入 NCCL_CUMEM_ENABLE=1 / NCCL_NVLS_ENABLE=1（fork 官方 inkling
    // recipe 要求；实测双 Spark 缺少这两条会在 NCCL 建连互等、vllm 卡在 "Reducing Torch threads"
    // 后无下文）。仅 IB 机型注入；extra_env 显式写了就用用户值（想回退 0 可自行覆盖）。
    if has_infiniband {
        if !extra_keys.contains("NCCL_CUMEM_ENABLE") {
            args.push("-e".to_string());
            args.push("NCCL_CUMEM_ENABLE=1".to_string());
        }
        if !extra_keys.contains("NCCL_NVLS_ENABLE") {
            args.push("-e".to_string());
            args.push("NCCL_NVLS_ENABLE=1".to_string());
        }
    }
    args
}

/// 把 vllm_args 拼接为 `--key value` 追加到 args；规则同 start_vllm_docker（仅非空/非默认值）。
/// `ctx_size`：设置页 launch_params.ctx_size（>0 时追加 `--max-model-len`，0/None = 模型自带默认）。
/// 全大写 KEY=VALUE 行视为环境变量误填，跳过并提示（避免误写成 --NCCL_DEBUG 之类 CLI 参数）。
fn push_vllm_args(args: &mut Vec<String>, vllm_args: &VllmArgs, ctx_size: Option<i32>) {
    if let Some(ctx) = ctx_size {
        if ctx > 0 {
            args.extend(["--max-model-len".to_string(), ctx.to_string()]);
        }
    }
    if vllm_args.tensor_parallel_size > 1 {
        args.extend(["--tensor-parallel-size".to_string(), vllm_args.tensor_parallel_size.to_string()]);
    }
    if vllm_args.gpu_memory_utilization > 0.0 {
        args.extend(["--gpu-memory-utilization".to_string(), format!("{}", vllm_args.gpu_memory_utilization)]);
    }
    if !vllm_args.quantization.is_empty() {
        args.extend(["--quantization".to_string(), vllm_args.quantization.clone()]);
    }
    if !vllm_args.kv_cache_dtype.is_empty() {
        args.extend(["--kv-cache-dtype".to_string(), vllm_args.kv_cache_dtype.clone()]);
    }
    if !vllm_args.reasoning_parser.is_empty() {
        args.extend(["--reasoning-parser".to_string(), vllm_args.reasoning_parser.clone()]);
    }
    if !vllm_args.tool_call_parser.is_empty() {
        args.extend(["--tool-call-parser".to_string(), vllm_args.tool_call_parser.clone()]);
    }
    if vllm_args.trust_remote_code {
        args.push("--trust-remote-code".to_string());
    }
    if vllm_args.enable_auto_tool_choice {
        args.push("--enable-auto-tool-choice".to_string());
    }
    if !vllm_args.distributed_executor_backend.is_empty() {
        args.extend(["--distributed-executor-backend".to_string(), vllm_args.distributed_executor_backend.clone()]);
    }
    if !vllm_args.load_format.is_empty() {
        args.extend(["--load-format".to_string(), vllm_args.load_format.clone()]);
    }
    if vllm_args.block_size > 0 {
        args.extend(["--block-size".to_string(), vllm_args.block_size.to_string()]);
    }
    if !vllm_args.tokenizer_mode.is_empty() {
        args.extend(["--tokenizer-mode".to_string(), vllm_args.tokenizer_mode.clone()]);
    }
    if vllm_args.max_num_seqs > 0 {
        args.extend(["--max-num-seqs".to_string(), vllm_args.max_num_seqs.to_string()]);
    }
    if vllm_args.max_num_batched_tokens != 0 {
        args.extend(["--max-num-batched-tokens".to_string(), vllm_args.max_num_batched_tokens.to_string()]);
    }
    for line in vllm_args.extra_args.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.split_once('=') {
            Some((k, v)) => {
                let k = k.trim().trim_start_matches("--");
                let v = v.trim();
                if !k.is_empty() && !v.is_empty() {
                    if k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') && k.len() > 1 {
                        crate::common::utils::logger::write_log(
                            "WARN",
                            "MODEL",
                            &format!("extra_args 疑似环境变量误填（已忽略，请填到多机「额外环境变量」）：{}={}", k, v),
                        );
                        continue;
                    }
                    args.push(format!("--{}", k));
                    args.push(v.to_string());
                }
            }
            None => {
                // 无等号：flag 型参数（如 disable-cuda-graph 或 --disable-cuda-graph）
                let k = line.trim_start_matches("--");
                if !k.is_empty() {
                    args.push(format!("--{}", k));
                }
            }
        }
    }
}

/// 把模型清单 vllm_flags 拼接为 `--key value` 追加到 args（每条 `--key value` 或 `--flag`，start_vllm_docker 同规则）。
fn push_vllm_flags(args: &mut Vec<String>, vllm_flags: &Option<Vec<String>>) {
    let flags = vllm_flags.as_deref().unwrap_or(&[]);
    if flags.is_empty() {
        return;
    }
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

/// 多机 head（本机 rank 0）docker run 参数：只跑 `sleep infinity`（fork keepalive）。
///
/// 严格对齐 eugr/spark-vllm-docker fork `launch-cluster.sh`：`start_cluster` 流程里
/// head 与 worker 一律先 `sleep infinity`，再分别通过 `docker exec -d` 起 Ray 进程。
/// Ray head 由 `start_multi_node` 本地 `docker exec -d` 触发（需在 vllm serve 之前 ready，
/// 否则 vllm 内部 ray.init() 找不到 cluster），vllm serve 由 `start_multi_node` 通过
/// 本地 `docker exec -i` 启动（替代历史方案的 `docker run` 直接 serve，更便于 inline
/// 触发 ray start --head、并捕获 stdout/stderr 经 child stdout pipe 转发到前端）。
///
/// 命令末尾端口/多机参数位置故意留空——这些是 vllm serve 的输入参数，由
/// `build_head_vllm_exec_args` 单独拼装。
fn build_multi_node_head_args(
    model_id: &str,
    model_dir: &std::path::Path,
    image: &str,
    shm_size: &str,
    mn: &MultiNodeArgs,
    node_ip: &str,
    iface: Option<&str>,
    ib_iface: Option<&str>,
    has_infiniband: bool,
    model_env: &[String],
) -> Vec<String> {
    let container_name = format!("adm-vllm-{}-rank-0", model_id);
    let mount_dst = format!("/models/{}", model_id);
    let mut args = build_common_docker_prefix(&container_name, shm_size, node_ip, iface, ib_iface, has_infiniband, &mn.extra_env, model_env);
    args.push("-v".to_string());
    args.push(format!("{}:{}:ro", model_dir.to_string_lossy(), mount_dst));
    // 容器必须 detached（本地回头要 `docker exec -d` 起 worker/head 的 vllm serve），`-d` 放 image 之前
    // 不再注入 RAY_ADDRESS：fork 镜像禁止 ray backend + nnodes>1，多机走 no-Ray(mp) 模式
    args.push("-d".to_string());
    // 对齐 build_multi_node_worker_args：清空镜像自带 ENTRYPOINT（如 vllm/vllm-openai 系的
    // ["vllm","serve"]），否则保活命令 `sleep infinity` 会被拼成 `vllm serve sleep infinity`（把 sleep 当模型名）
    args.push("--entrypoint=".to_string());
    args.push(image.to_string());
    args.push("sleep".to_string());
    args.push("infinity".to_string());
    args
}

/// 多机 head 本地 `docker exec -i <container> bash -c "..."` 内部 `vllm serve ...` 命令的参数。
/// 末尾固定追加 fork `launch-cluster.sh exec_no_ray_cluster` 的 head 多机参数：
///   `--distributed-executor-backend mp --nnodes N --node-rank 0 --tensor-parallel-size N
///    --master-addr <node0_ip> --master-port <dist_init_port>`
/// 注意：fork 镜像 pydantic 校验 `nnodes > 1 can only be set when distributed executor
/// backend is mp, uni or external_launcher`——因此 backend 必须显式 mp（覆盖用户 vllm_flags
/// 里可能自带的 ray），绝不能再用 ray。
/// 调用方负责 shell escape 并拼成 `bash -c "<joined tokens>"`。
fn build_head_vllm_exec_args(
    model_id: &str,
    port: u16,
    vllm_args: &VllmArgs,
    vllm_flags: &Option<Vec<String>>,
    mn: &MultiNodeArgs,
    ctx_size: Option<i32>,
) -> Vec<String> {
    let mount_dst = format!("/models/{}", model_id);
    let node0_ip = mn.nodes[0].ip.clone();
    let mut args = vec![
        "vllm".to_string(),
        "serve".to_string(),
        mount_dst,
        "--host".to_string(),
        "0.0.0.0".to_string(),
        "--port".to_string(),
        port.to_string(),
    ];
    push_vllm_args(&mut args, vllm_args, ctx_size);
    push_vllm_flags(&mut args, vllm_flags);
    let n_nodes = mn.nodes.len();
    args.extend([
        "--distributed-executor-backend".to_string(),
        "mp".to_string(),
        "--nnodes".to_string(),
        n_nodes.to_string(),
        "--node-rank".to_string(),
        "0".to_string(),
        "--tensor-parallel-size".to_string(),
        n_nodes.to_string(),
        "--master-addr".to_string(),
        node0_ip,
        "--master-port".to_string(),
        mn.dist_init_port.to_string(),
    ]);
    args
}

/// 多机 worker（远端 rank i）vllm serve 命令参数：与 head 相同但末尾追加 `--headless`。
/// 严格对齐 fork `exec_no_ray_cluster`：每条 worker 都完整起一个 vllm serve（不 serve API，
/// 仅参与分布式初始化），master 地址指向 head（node 0），由 head 端等待所有 rank join。
fn build_multi_node_worker_vllm_args(
    model_id: &str,
    port: u16,
    vllm_args: &VllmArgs,
    vllm_flags: &Option<Vec<String>>,
    rank: usize,
    mn: &MultiNodeArgs,
    ctx_size: Option<i32>,
) -> Vec<String> {
    let mount_dst = format!("/models/{}", model_id);
    let node0_ip = mn.nodes[0].ip.clone();
    let mut args = vec![
        "vllm".to_string(),
        "serve".to_string(),
        mount_dst,
        "--host".to_string(),
        "0.0.0.0".to_string(),
        "--port".to_string(),
        port.to_string(),
    ];
    push_vllm_args(&mut args, vllm_args, ctx_size);
    push_vllm_flags(&mut args, vllm_flags);
    let n_nodes = mn.nodes.len();
    args.extend([
        "--distributed-executor-backend".to_string(),
        "mp".to_string(),
        "--nnodes".to_string(),
        n_nodes.to_string(),
        "--node-rank".to_string(),
        rank.to_string(),
        "--tensor-parallel-size".to_string(),
        n_nodes.to_string(),
        "--master-addr".to_string(),
        node0_ip,
        "--master-port".to_string(),
        mn.dist_init_port.to_string(),
        "--headless".to_string(),
    ]);
    args
}

/// 多机 worker（远端 rank i）docker run 参数：跑 `sleep infinity`（fork `launch-cluster.sh` keepalive）。
///
/// 严格对齐 eugr/spark-vllm-docker fork `launch-cluster.sh --ray` 模式：
/// 1. 容器仅跑 `sleep infinity`，不做任何业务逻辑
/// 2. Ray 进程由本函数返回 docker run 完成后，由 `start_multi_node` 通过 SSH `docker exec -d` 启动
///    `ray start --block --address=<head>:<port> ...` 加入 head 的 Ray 集群
/// 3. 这避免了项目历史方案 `python -m vllm.distributed.ray_utils` 的两个隐患：
///    - 镜像无 `python` 软链导致 nvidia_entrypoint.sh 报 `exec: python: not found`
///    - `vllm.distributed.ray_utils` 模块并未出现在 fork 公开仓库里（依赖隐性维护分支）
///
/// Ray 启动参数与 fork `start_ray_worker` 完全一致：
///   ray start --block --object-store-memory=1073741824 --num-cpus=2 --disable-usage-stats
///            --address=<head_ip>:<dist_init_port> --node-ip-address=<worker_ip>
fn build_multi_node_worker_args(
    model_id: &str,
    model_dir: &std::path::Path,
    image: &str,
    shm_size: &str,
    rank: usize,
    _node0_ip: &str,
    mn: &MultiNodeArgs,
    node_ip: &str,
    iface: Option<&str>,
    ib_iface: Option<&str>,
    has_infiniband: bool,
    model_env: &[String],
) -> Vec<String> {
    let container_name = format!("adm-vllm-{}-rank-{}", model_id, rank);
    let mount_dst = format!("/models/{}", model_id);
    let mut args = build_common_docker_prefix(&container_name, shm_size, node_ip, iface, ib_iface, has_infiniband, &mn.extra_env, model_env);
    args.push("-v".to_string());
    args.push(format!("{}:{}:ro", model_dir.to_string_lossy(), mount_dst));
    // fork 默认 `--entrypoint=` 清空镜像 ENTRYPOINT（避免 nvidia_entrypoint.sh 触发），
    // 命令仅 `sleep infinity`，让容器保活等待后续 docker exec 启动 Ray worker
    args.push("--entrypoint=".to_string());
    args.push(image.to_string());
    args.push("sleep".to_string());
    args.push("infinity".to_string());
    args
}

/// 本机（rank 0）docker run 容器名（多机停止时据此识别多机模式）
fn multi_container_name(model_id: &str, rank: usize) -> String {
    format!("adm-vllm-{}-rank-{}", model_id, rank)
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
    vllm_image: Option<String>,
    vllm_flags: Option<Vec<String>>,
    vllm_env: Option<Vec<String>>,
) -> Result<(), AppError> {
    let (mn, mut vllm_args) = load_multi_node_config(app);
    // 多机路径：backend 由下方 build_head/worker_vllm_exec_args 末尾强制追加 mp，
    // 清除残留值避免 push_vllm_args 先推一个旧值再被 mp 覆盖（产生重复 flag）。
    vllm_args.distributed_executor_backend.clear();
    let port: u16 = params.port.unwrap_or(8000);
    validate_multi_node(&mn, port)?;
    let n_nodes = mn.nodes.len();
    let key: Option<String> = if mn.ssh_key_path.trim().is_empty() {
        None
    } else {
        Some(mn.ssh_key_path.trim().to_string())
    };
    let key_ref = key.as_deref();

    // 本地 vllm 日志落盘：<data_dir>/logs/adm_vllm_<model_id>_rank_0.log
    // vllm 每行输出实时写文件（每行 flush），容器被清理后日志依然可查——
    // 排查"启动即失败被 docker rm 删掉、原因无处可看"的关键。
    let local_vllm_log = crate::common::config::get_data_dir(Some(app))?
        .join("logs")
        .join(format!("adm_vllm_{}_rank_0.log", model_id));
    if let Some(parent) = local_vllm_log.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let log_writer = std::sync::Arc::new(std::sync::Mutex::new(
        std::io::BufWriter::new(
            std::fs::File::create(&local_vllm_log)
                .map_err(|e| AppError::msg(format!("创建 vllm 日志文件失败（{}）: {}", local_vllm_log.display(), e)))?,
        ),
    ));

    // 镜像由远程 model.json 的 vllm_image 字段唯一指定（每模型独立配置），缺字段视为清单错误。
    let image = vllm_image
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::msg(format!("模型 {} 缺少 vllm_image 配置（远程 model.json 必填）", model_id)))?
        .to_string();
    let default_shm = "64g";
    let shm_size = if vllm_args.shm_size.is_empty() { default_shm.to_string() } else { vllm_args.shm_size.clone() };

    // 本机 Docker 预检（CLI / daemon / GPU runtime / 镜像，含自动 pull）
    let image = check_docker_env(app, model_id, &image).await?;
    // 本机端口占用检查（--network host 下端口直接 bind 宿主机）
    {
        let probe = std::net::TcpListener::bind(("0.0.0.0", port));
        if probe.is_err() {
            bail!("端口 {} 已被占用，请先关闭占用该端口的进程，或在设置页更换监听端口", port);
        }
    }
    // master 端口占用检查（worker 通过该端口与 head 建立分布式连接）
    {
        let probe = std::net::TcpListener::bind(("0.0.0.0", mn.dist_init_port));
        if probe.is_err() {
            bail!("多机 master 端口 {} 已被占用，请在设置页更换 master 端口", mn.dist_init_port);
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
    // 收集各 worker 的 (rank, container, use_sudo, log_path)，供 Phase 5 复用——
    // 避免再对每个节点重复 SSH 探测 sudo（SSH 抖动可能导致 sudo 判定与容器启动时不一致）
    let mut worker_runtime: Vec<(usize, String, bool, String)> = Vec::new();
    for (i, node) in mn.nodes.iter().enumerate().skip(1) {
        let node_model_dir = effective_remote_model_dir(&node.model_dir, &node.ssh_user, model_id);
        // 预检：docker daemon / GPU / 本机所用镜像 / 模型目录（含 .done）
        let probe = crate::common::ssh::probe_script(&node_model_dir, &image, false, true);
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

        // 启动：nohup docker run 后台运行，日志落盘 /tmp/adm_vllm_<model>_rank_<i>.log
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
        // 检测远端是否有 /dev/infiniband（DGX Spark 有 IB 网卡，EdgeXpert 等无）
        let (_, ib_out, _) = crate::common::ssh::ssh_run(
            &node.ip, &node.ssh_user, node.ssh_port, key_ref,
            "test -d /dev/infiniband && echo IB_YES || echo IB_NO",
            std::time::Duration::from_secs(10),
        ).await.unwrap_or((false, String::new(), String::new()));
        let remote_has_ib = ib_out.contains("IB_YES");
        if remote_has_ib {
            let _ = app.emit("model-log", serde_json::json!({
                "model_id": model_id,
                "line": format!("[多机] 节点 {}（rank {}）检测到 InfiniBand 设备", node.ip, i),
                "source": "stdout",
            }));
        }
        // 远端 IB 接口名（NCCL_IB_HCA 用）：取 ib 口名，如 mlx5_0；拿不到则留空让 NCCL 自检
        let remote_ib_iface = if remote_has_ib {
            crate::common::ssh::ssh_run(
                &node.ip, &node.ssh_user, node.ssh_port, key_ref,
                "ls /sys/class/infiniband 2>/dev/null | head -3 | tr '\\n' ',' | sed 's/,$//'",
                std::time::Duration::from_secs(10),
            ).await.ok().map(|(_, o, _)| o.trim().to_string()).filter(|s| !s.is_empty() && s != "No such file or directory")
        } else {
            None
        };
        let args = build_multi_node_worker_args(
            model_id,
            &std::path::Path::new(&node_model_dir),
            &image,
            &shm_size,
            i,
            &node0_ip,
            &mn,
            &node.ip,
            remote_iface.as_deref(),
            remote_ib_iface.as_deref(),
            remote_has_ib,
            vllm_env.as_deref().unwrap_or(&[]),
        );
        let container = multi_container_name(model_id, i);
        let log_path = format!("/tmp/adm_vllm_{}_rank_{}.log", model_id, i);
        // 清理远端同名残留容器（上次启动失败/手动残留，与 rank 0 的 docker rm -f 对齐），
        // 否则 docker run --name 立即失败且不会出现在 docker ps 中
        let clean_script = crate::common::ssh::stop_container_script(&container);
        let _ = crate::common::ssh::ssh_run(
            &node.ip, &node.ssh_user, node.ssh_port, key_ref, &clean_script,
            std::time::Duration::from_secs(20),
        ).await;
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
        // 注意需用 docker ps -a 轮询：容器 Exited 后普通 docker ps 不再列出，漏检会误报「未进入运行状态」
        let mut up = false;
        let mut tail = String::new();
        let mut poll_fail = 0usize;
        for _ in 0..15 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let chk = format!("(sudo -n docker ps -a --filter name={} --format '{{{{.Names}}}} {{{{.Status}}}}' 2>/dev/null || docker ps -a --filter name={} --format '{{{{.Names}}}} {{{{.Status}}}}' 2>/dev/null)", crate::common::ssh::sh_quote(&container), crate::common::ssh::sh_quote(&container));
            match crate::common::ssh::ssh_run(
                &node.ip, &node.ssh_user, node.ssh_port, key_ref, &chk,
                std::time::Duration::from_secs(10),
            ).await {
                Ok((_, out2, _)) => {
                    if out2.contains(&container) && out2.contains("Up") {
                        up = true;
                        break;
                    }
                    // Exited / Dead / Restarting（崩溃循环）都是启动失败，抓日志尾部定位原因
                    if out2.contains("Exited") || out2.contains("Dead") || out2.contains("Restarting") {
                        if let Ok((_, t2, _)) = crate::common::ssh::ssh_run(
                            &node.ip, &node.ssh_user, node.ssh_port, key_ref,
                            &format!("tail -n 50 {}", crate::common::ssh::sh_quote(&log_path)),
                            std::time::Duration::from_secs(10),
                        ).await {
                            tail = t2;
                        }
                        break;
                    }
                }
                Err(_) => poll_fail += 1,
            }
        }
        if !up {
            let _ = stop_remote_containers(app, &mn, key_ref, model_id, 1).await;
            // 兜底抓取容器状态与日志尾部，给出真实失败原因（而非笼统「未进入运行状态」）
            let ps_a = crate::common::ssh::ssh_run(
                &node.ip, &node.ssh_user, node.ssh_port, key_ref,
                &format!("(sudo -n docker ps -a --filter name={} --format '{{{{.Names}}}} {{{{.Status}}}}' 2>/dev/null || docker ps -a --filter name={} --format '{{{{.Names}}}} {{{{.Status}}}}' 2>/dev/null)", crate::common::ssh::sh_quote(&container), crate::common::ssh::sh_quote(&container)),
                std::time::Duration::from_secs(10),
            ).await.ok().map(|(_, o, _)| o.trim().to_string()).filter(|o| !o.is_empty());
            if tail.trim().is_empty() {
                if let Ok((_, t2, _)) = crate::common::ssh::ssh_run(
                    &node.ip, &node.ssh_user, node.ssh_port, key_ref,
                    &format!("tail -n 50 {}", crate::common::ssh::sh_quote(&log_path)),
                    std::time::Duration::from_secs(10),
                ).await {
                    tail = t2;
                }
            }
            let detail = if !tail.trim().is_empty() {
                tail
            } else if let Some(st) = ps_a {
                format!("容器状态：{}（日志为空，docker run 阶段即失败，可在远端查看 /tmp/adm_vllm_{}_rank_{}.log）", st, model_id, i)
            } else {
                let hint = if poll_fail > 0 { format!("远端 SSH 轮询失败 {} 次；", poll_fail) } else { String::new() };
                format!("{}30s 内未进入运行状态（未看到容器，docker run 可能立即失败，可在远端查看 /tmp/adm_vllm_{}_rank_{}.log）", hint, model_id, i)
            };
            crate::common::utils::logger::write_log("ERROR", "MODEL", &format!("[{}] 远端节点 {} 容器启动失败:\n{}", model_id, node.ip, detail));
            bail!("远端节点 {}（rank {}）容器启动失败，已回滚停止已启动节点：\n{}", node.ip, i, detail);
        }
        let _ = app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": format!("[多机] 远端节点 {}（rank {}）容器已就绪", node.ip, i),
            "source": "stdout",
        }));
        // worker 容器内 vllm 可用性预检：worker 侧 vllm 缺失时其 vllm serve 秒退，
        // head 会一直阻塞等 rank join（前端只见超时）。提前探测并失败回滚。
        // 失败时同时回传容器内 PATH 与 vllm 常见安装位置（定位镜像是否为旧构建/venv 布局）。
        {
            let probe_cmd = format!(
                "{} exec {} bash -c 'if command -v vllm >/dev/null 2>&1; then echo VLLM_BIN_OK; else echo VLLM_BIN_MISSING; echo \"PATH=$PATH\"; command -v sglang >/dev/null 2>&1 && echo IMAGE_IS_SGLANG; ls /usr/local/bin /usr/bin /opt/venv/bin /opt/conda/bin 2>/dev/null | grep -i vllm | head -5; fi'",
                if use_sudo { "sudo -n docker" } else { "docker" },
                crate::common::ssh::sh_quote(&container)
            );
            let (pok, pout, perr) = crate::common::ssh::ssh_run(
                &node.ip, &node.ssh_user, node.ssh_port, key_ref, &probe_cmd,
                std::time::Duration::from_secs(15),
            ).await.unwrap_or((false, String::new(), String::new()));
            if !pok || pout.contains("VLLM_BIN_MISSING") {
                let _ = stop_remote_containers(app, &mn, key_ref, model_id, 1).await;
                let diag: Vec<&str> = pout.lines()
                    .filter(|l| l.contains("PATH=") || l.to_lowercase().contains("vllm") || l.contains("IMAGE_IS_SGLANG"))
                    .collect();
                let diag_str = if diag.is_empty() { String::new() } else { format!("（{}\n）", diag.join("\n")) };
                let is_sglang = pout.contains("IMAGE_IS_SGLANG");
                let sglang_hint = if is_sglang {
                    "该镜像内是 sglang 而非 vllm（tag 实际被 SGLang 镜像占用/打错），请改用真正的 vLLM 镜像"
                } else {
                    "镜像可能为旧构建或经镜像加速器缓存（浮动 tag 内容不一致）"
                };
                let detail = if pout.contains("VLLM_BIN_MISSING") {
                    format!("容器内找不到 vllm 命令。{}。{}", sglang_hint, diag_str)
                } else {
                    "容器内 vllm 预检执行失败".to_string()
                };
                crate::common::utils::logger::write_log("ERROR", "MODEL", &format!("[{}] {}（节点 {}）: {}", model_id, detail, node.ip, perr));
                bail!(
                    "远端节点 {}（rank {}）{}。镜像可能为旧构建或经镜像加速器缓存（浮动 tag 内容不一致）：请在该节点执行 `docker rmi {}` 后，本机设置页「同步镜像到直连节点」重新推送（或远端 docker pull），已回滚停止已启动节点",
                    node.ip, i, detail, image
                );
            }
        }
        // 记录 (rank, container, use_sudo, log_path) 供 Phase 5 ray join 复用（避免重复 sudo 探测）
        worker_runtime.push((i, container.clone(), use_sudo, log_path));
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
    let local_has_ib = std::path::Path::new("/dev/infiniband").exists();
    // 本机 IB 接口名（NCCL_IB_HCA 用）
    let local_ib_iface = if local_has_ib {
        std::process::Command::new("sh")
            .args(["-c", "ls /sys/class/infiniband 2>/dev/null | head -3 | tr '\\n' ',' | sed 's/,$//'"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        None
    };
    let args0 = build_multi_node_head_args(
        model_id, model_dir, &image, &shm_size, &mn, &node0_ip, local_iface.as_deref(), local_ib_iface.as_deref(), local_has_ib,
        vllm_env.as_deref().unwrap_or(&[]),
    );

    dbg_log!("vllm multi-node head container args (rank0): {:?}", args0);
    let _ = app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("[多机] 本机（rank 0）启动保活容器 {}（sleep infinity）", container0),
        "source": "stdout",
    }));

    // ===== Phase 1: 启动 head 保活容器（detached -d）=====
    // 容器只跑 sleep infinity；后续两个 docker exec 步骤注入 Ray head + vllm serve。
    // 这样避开镜像无 python 软链导致的 ENTRYPOINT 问题（fork launch-cluster.sh 标准做法）。
    if let Ok(o) = crate::common::utils::platform::docker_cmd()
        .args(&args0)
        .output() {
        if !o.status.success() {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let _ = app.emit("model-log", serde_json::json!({
                "model_id": model_id,
                "line": format!("[ERROR] head 容器启动失败: {}", stderr.trim()),
                "source": "stderr",
            }));
            // 回滚：移除残留 head 容器 + 远端 worker 容器
            let _ = crate::common::utils::platform::docker_cmd()
                .args(["rm", "-f", &container0])
                .output();
            stop_remote_containers(app, &mn, key_ref, model_id, 1).await;
            return Err(AppError::msg(format!("head 容器启动失败: {}", stderr.trim())));
        }
    } else {
        let _ = crate::common::utils::platform::docker_cmd()
            .args(["rm", "-f", &container0])
            .output();
        stop_remote_containers(app, &mn, key_ref, model_id, 1).await;
        return Err(AppError::msg("本地 docker run 失败".to_string()));
    }

    // ===== Phase 2: 等待 head 容器 Up =====
    // 本地 `std::process::Command` 直传参数（不经 shell）：`--filter name=<container>` 里的容器名
    // 是合法 docker 名（无空格/引号），不能套 sh_quote——单引号会原样传给 docker 导致永远匹配不上。
    let ps_filter0 = format!("name={}", container0);
    let mut head_up = false;
    for _ in 0..15 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if let Ok(o) = crate::common::utils::platform::docker_cmd()
            .args(["ps", "-a", "--filter", &ps_filter0, "--format", "{{.Names}} {{.Status}}"])
            .output() {
            let s = String::from_utf8_lossy(&o.stdout);
            if s.contains(&container0) && s.contains("Up") {
                head_up = true;
                break;
            }
            // Exited 立即报错（镜像缺失/参数错一般秒退）
            if s.contains("Exited") || s.contains("Dead") {
                break;
            }
        }
    }
    if !head_up {
        // 抓取 docker ps 输出以便诊断
        let ps_out = crate::common::utils::platform::docker_cmd()
            .args(["ps", "-a", "--filter", &ps_filter0, "--format", "{{.Names}} {{.Status}}"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let _ = app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": format!("[ERROR] head 容器 30s 内未进入 Up：{}", ps_out.trim()),
            "source": "stderr",
        }));
        let _ = crate::common::utils::platform::docker_cmd()
            .args(["rm", "-f", &container0])
            .output();
        stop_remote_containers(app, &mn, key_ref, model_id, 1).await;
        return Err(AppError::msg("head 容器 30s 内未进入 Up 状态".to_string()));
    }
    let _ = app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("[多机] 本机（rank 0）容器已就绪"),
        "source": "stdout",
    }));

    // ===== Phase 2.5: head 容器内 vllm 可用性预检 =====
    // 镜像 PATH 缺 vllm 时，Phase 4 的 `bash -c "vllm serve ..."` 只会留下
    // `bash: line 1: vllm: command not found` 然后 head 阻塞等 rank join——提前探测，
    // 失败即回滚并给出可操作的修复指引（重拉镜像）。顺带检测 sglang 占 tag 场景。
    {
        let probe = crate::common::utils::platform::docker_cmd()
            .args(["exec", &container0, "bash", "-c",
                   "if command -v vllm >/dev/null 2>&1; then echo VLLM_BIN_OK; else echo VLLM_BIN_MISSING; command -v sglang >/dev/null 2>&1 && echo IMAGE_IS_SGLANG; fi"])
            .output();
        if let Ok(o) = &probe {
            let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
            if stdout.contains("VLLM_BIN_MISSING") {
                let hint = if stdout.contains("IMAGE_IS_SGLANG") {
                    "（该镜像内是 sglang 而非 vllm：此 tag 实际被 SGLang 镜像占用/打错，请改用真正的 vLLM 镜像，如 ghcr.io/spark-arena/dgx-vllm-eugr-nightly-b12x）"
                } else {
                    ""
                };
                let diag = crate::common::utils::platform::docker_cmd()
                    .args(["exec", &container0, "bash", "-c",
                           "echo PATH=$PATH; ls /usr/local/bin /usr/bin 2>/dev/null | grep -i vllm | head -3"])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                let _ = app.emit("model-log", serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[ERROR] 镜像 {} 容器内找不到 vllm 命令{}（{}）", image, hint, if diag.is_empty() { "容器内无 vllm 相关文件".to_string() } else { diag }),
                    "source": "stderr",
                }));
                let _ = crate::common::utils::platform::docker_cmd()
                    .args(["rm", "-f", &container0])
                    .output();
                stop_remote_containers(app, &mn, key_ref, model_id, 1).await;
                return Err(AppError::msg(format!(
                    "镜像 {} 容器内找不到 vllm 命令{}（镜像内容可能不完整或被镜像加速器替换）：请执行 `docker rmi {}` 后在模型列表重新下载模型触发重新拉取，已回滚停止集群",
                    image, hint, image
                )));
            }
        }
    }

    // ===== Phase 3: 派发远端 worker 的 `vllm serve --headless`（no-Ray 多机，对齐 fork exec_no_ray_cluster）=====
    // fork 镜像 pydantic 校验：nnodes > 1 只允许 mp / uni / external_launcher backend，
    // 因此 worker 与 head 都用 `--distributed-executor-backend mp`（build 函数末尾强推，覆盖
    // 用户 vllm_flags 里可能自带的 ray）。worker 必须先于 head 启动（但 docker exec -d 后台，
    // 只派发不等待；head 的 vllm serve 会阻塞等所有 rank join）。
    let _ = app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("[多机] 派发远端 worker 的 vllm serve --headless（rank 1..{}）", n_nodes - 1),
        "source": "stdout",
    }));
    for (i, container, use_sudo, _log_path) in &worker_runtime {
        let node = &mn.nodes[*i];
        let worker_vllm_args = build_multi_node_worker_vllm_args(
            model_id, port, &vllm_args, &vllm_flags, *i, &mn, params.ctx_size,
        );
        let start_script = crate::common::ssh::start_vllm_worker_script(
            container, &worker_vllm_args, *use_sudo,
        );
        match crate::common::ssh::ssh_run(
            &node.ip, &node.ssh_user, node.ssh_port, key_ref, &start_script,
            std::time::Duration::from_secs(15),
        ).await {
            Ok((true, _, _)) => {
                let _ = app.emit("model-log", serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[多机] 远端节点 {}（rank {}）vllm serve --headless 已派发", node.ip, *i),
                    "source": "stdout",
                }));
            }
            Ok((false, _, err)) => {
                crate::common::utils::logger::write_log(
                    "WARN", "MODEL",
                    &format!("[{}] 远端节点 {}（rank {}）vllm worker 派发失败: {}", model_id, node.ip, *i, err),
                );
                let _ = app.emit("model-log", serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[多机] 远端节点 {}（rank {}）vllm worker 派发失败（继续，head 端可能超时）", node.ip, *i),
                    "source": "stderr",
                }));
            }
            Err(e) => {
                crate::common::utils::logger::write_log(
                    "WARN", "MODEL",
                    &format!("[{}] 远端节点 {}（rank {}）vllm worker 派发异常: {}", model_id, node.ip, *i, e),
                );
            }
        }
    }

    // ===== Phase 4: 在 head 容器内 exec vllm serve（本地 docker exec -i 捕获 stdout）=====
    // 用 build_head_vllm_exec_args 拼 vllm serve 的命令 tokens；经 sh_quote + bash -c 串成一行，
    // 整段塞进 `docker exec -i <head_container> bash -c "<cmd>"`，child stdout/stderr 仍归
    // 我们管（沿用原 spawn_docker_run 的转发链路，避免再换 docker logs -f 拉一条新轮询线程）。
    let vllm_exec_args = build_head_vllm_exec_args(
        model_id, port, &vllm_args, &vllm_flags, &mn, params.ctx_size,
    );
    let vllm_inner_cmd: String = vllm_exec_args.iter()
        .map(|s| crate::common::ssh::sh_quote(s))
        .collect::<Vec<_>>()
        .join(" ");
    // 双通道输出：`vllm ... 2>&1 | tee /proc/1/fd/1`
    //  - tee 的 stdout 进 docker exec 的 stdout pipe（我们的 child 转发线程 → 前端日志）
    //  - tee 同时写 `/proc/1/fd/1`（容器 PID 1 的 stdout → `docker logs <head>` 可见）
    // 这样 vllm 启动即失败（参数解析/镜像不兼容等）时，失败原因在 docker logs 里也有，
    // 不会再出现"全部日志就这些然后容器被清理"、原因不可见的情况。
    let vllm_inner = format!("{} 2>&1 | tee /proc/1/fd/1", vllm_inner_cmd);
    let _ = app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("[多机] 本机（rank 0）启动 vllm serve（docker exec -i，mp 多机模式）"),
        "source": "stdout",
    }));
    let mut child = {
        let mut cmd = crate::common::utils::platform::docker_cmd();
        cmd.arg("exec")
            .arg("-i")
            .arg("-e").arg("TERM=xterm")
            .arg(&container0)
            .arg("bash").arg("-c").arg(&vllm_inner)
            // 显式 stdin null：`docker exec -i` 默认继承父进程 stdin，
            // Windows 无控制台（windows_subsystem）下可能拿到失效句柄导致 docker 报错
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("启动 vllm serve 失败: {}", e);
                let _ = app.emit("model-log", serde_json::json!({
                    "model_id": model_id, "line": format!("[ERROR] {}", msg), "source": "stderr",
                }));
                let _ = crate::common::utils::platform::docker_cmd()
                    .args(["rm", "-f", &container0]).output();
                stop_remote_containers(app, &mn, key_ref, model_id, 1).await;
                return Err(AppError::msg(msg));
            }
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
    let log_writer_stdout = std::sync::Arc::clone(&log_writer);
    let log_writer_stderr = std::sync::Arc::clone(&log_writer);

    // 提示用户本地日志路径（便于事后排查）
    let _ = app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("[多机] vllm 日志实时落盘：{}", local_vllm_log.display()),
        "source": "stdout",
    }));

    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Write};

        let stdout_handle = if let Some(stdout) = child.stdout.take() {
            let app_c = app_clone.clone();
            let mid = model_id_clone.clone();
            let lw = std::sync::Arc::clone(&log_writer_stdout);
            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    crate::common::utils::logger::write_log("INFO", "vLLM", &line);
                    if let Ok(mut w) = lw.lock() {
                        let _ = writeln!(w, "{}", line);
                        let _ = w.flush();
                    }
                    app_c.emit("model-log", serde_json::json!({
                        "model_id": &mid, "line": line.clone(), "source": "stdout",
                    })).ok();
                    if line.contains("Uvicorn running on")
                        || line.contains("Application startup complete")
                        || line.contains("Starting vLLM API server")
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
            let lw = std::sync::Arc::clone(&log_writer_stderr);
            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    crate::common::utils::logger::write_log("WARN", "vLLM", &line);
                    if let Ok(mut w) = lw.lock() {
                        let _ = writeln!(w, "{}", line);
                        let _ = w.flush();
                    }
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

        // vllm 已退出（可能是启动即失败）：删除容器前抓取 docker logs 尾部，
        // 把 vllm 侧真实报错转发到前端（vllm 输出已 tee 到容器 stdout），
        // 避免"容器被清理 + 失败原因无处可查"。
        if let Ok(o) = crate::common::utils::platform::docker_cmd()
            .args(["logs", "--tail", "60", &container_clone0])
            .output()
        {
            let tail = String::from_utf8_lossy(&o.stdout)
                .lines()
                .chain(String::from_utf8_lossy(&o.stderr).lines())
                .collect::<Vec<_>>()
                .join("\n");
            if !tail.trim().is_empty() {
                crate::common::utils::logger::write_log("ERROR", "vLLM", &tail);
                app_clone2.emit("model-log", serde_json::json!({
                    "model_id": &model_id_clone2,
                    "line": format!("[vllm exited] 容器日志尾部（清理前抓取）:\n{}", tail),
                    "source": "stderr",
                })).ok();
            }
        }

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
                    let chk = format!("(sudo -n docker ps --filter name={} --format '{{{{.Names}}}} {{{{.Status}}}}' 2>/dev/null || docker ps --filter name={} --format '{{{{.Names}}}} {{{{.Status}}}}' 2>/dev/null)", crate::common::ssh::sh_quote(&c), crate::common::ssh::sh_quote(&c));
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
                            let log_path = format!("/tmp/adm_vllm_{}_rank_{}.log", mid, i);
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
                        Ok((_, out, _)) if out.contains("Up") => {
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

/// vLLM Docker 启动（Ubuntu / DGX Spark 等机型）。
/// 模型目录以只读方式挂载进容器，容器前台运行（生命周期 = docker run 进程），
/// 就绪信号：stdout 出现 "Application startup complete" / "Starting vLLM API server" / "Uvicorn running on"。
async fn start_vllm_docker(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
    model_id: &str,
    model_dir: &std::path::Path,
    params: LaunchParams,
    device: Option<String>,
    vllm_image: Option<String>,
    vllm_flags: Option<Vec<String>>,
    vllm_env: Option<Vec<String>>,
) -> Result<(), AppError> {
    const CONTAINER_PREFIX: &str = "adm-vllm-";
    let container_name = format!("{}{}", CONTAINER_PREFIX, model_id);

    // 加载设置中的 vLLM 详细参数配置
    let settings_path = config::get_data_dir(Some(app))?.join("config.json");
    let mut vllm_args = VllmArgs::default();
    if let Ok(json) = std::fs::read_to_string(&settings_path) {
        if let Ok(parsed) = serde_json::from_str::<Settings>(&json) {
            vllm_args = parsed.vllm_args;
        }
    }
    // 单机路径：分布式后端由 vllm 自动选择（mp），忽略设置中可能残留的 ray/mp 旧值——
    // 多机模式由 start_multi_node 强制 mp，用户无需（也不应）手动配置分布式后端。
    vllm_args.distributed_executor_backend.clear();

    // 镜像由远程 model.json 的 vllm_image 字段唯一指定（每模型独立配置），本地不再保留硬编码兜底；
    // 缺字段视为模型清单配置错误，直接拒绝启动。
    let image = vllm_image
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::msg(format!("模型 {} 缺少 vllm_image 配置（远程 model.json 必填）", model_id)))?
        .to_string();
    crate::common::utils::logger::write_log("INFO", "DOCKER", &format!("[{}] 使用镜像 {}", model_id, image));
    app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("使用镜像 {}（模型 vllm_image）", image),
        "source": "stdout",
    })).ok();
    let default_shm = match device.as_deref() {
        Some("dgx-spark-128G") => "64g",
        _ => "32g",
    };
    let shm_size = if vllm_args.shm_size.is_empty() { default_shm.to_string() } else { vllm_args.shm_size.clone() };

    // ===== 启动前 Docker 环境预检（CLI / daemon / GPU runtime / 镜像）=====
    // 返回实际可用镜像名（可能是国内镜像源前缀版本），后续 docker run 必须用它
    let image = check_docker_env(app, model_id, &image).await?;

    // 镜像内 vllm 可用性预检（--entrypoint bash 绕开镜像 ENTRYPOINT，无需 GPU）：
    // vllm 不在镜像 PATH 时容器会以 `bash: line 1: vllm: command not found` 或
    // OCI exec 错误秒退，提前探测给出可操作的修复指引（重拉镜像）。
    // 同时顺带检测 sglang：tagname 可能被非 vLLM 镜像（如 SGLang）占用，报明避免误判镜像损坏。
    {
        let probe = crate::common::utils::platform::docker_cmd_tokio()
            .args(["run", "--rm", "--entrypoint", "/bin/bash", &image, "-c",
                   "if command -v vllm >/dev/null 2>&1; then echo VLLM_BIN_OK; else echo VLLM_BIN_MISSING; command -v sglang >/dev/null 2>&1 && echo IMAGE_IS_SGLANG; fi"])
            .output();
        if let Ok(Ok(o)) = tokio::time::timeout(std::time::Duration::from_secs(60), probe).await {
            let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
            if stdout.contains("VLLM_BIN_MISSING") {
                let hint = if stdout.contains("IMAGE_IS_SGLANG") {
                    "（检测到该镜像内是 sglang 而非 vllm：此 tag 实际被 SGLang 镜像占用/打错，请改用真正的 vLLM 镜像，如本清单其他模型使用的 ghcr.io/spark-arena/dgx-vllm-eugr-nightly-b12x）"
                } else {
                    ""
                };
                let msg = format!(
                    "镜像 {} 内找不到 vllm 命令{}（镜像内容可能不完整或被镜像加速器替换）：请执行 `docker rmi {}` 后在模型列表重新下载模型触发重新拉取",
                    image, hint, image
                );
                app.emit("model-log", serde_json::json!({
                    "model_id": model_id, "line": format!("[ERROR] {}", msg), "source": "stderr",
                })).ok();
                bail!("{}", msg);
            }
        }
    }

    let port: u16 = params.port.unwrap_or(8000);
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

    // ===== 设置页「额外环境变量」注入（与多机 build_common_docker_prefix 行为对齐）=====
    // 每行 KEY=VALUE → -e KEY=VALUE；extra_env 已显式设置 NCCL_DEBUG 时不覆盖，未设置时默认 INFO。
    // KEY 全大写+数字+下划线 + 长度 > 1 才视为合法（避免误把空行/残行注入）。
    let mut extra_env_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut extra_env_args: Vec<String> = Vec::new();
    for line in vllm_args.extra_env.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            if !k.is_empty() && !v.is_empty() && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') && k.len() > 1 {
                extra_env_keys.insert(k.to_string());
                extra_env_args.push("-e".to_string());
                extra_env_args.push(format!("{}={}", k, v));
            }
        }
    }
    // ===== 模型清单 vllm_env 注入（docker 同名 -e 后者生效 → 模型清单优先级最高，与 vllm_flags 语义一致）=====
    // 每条 KEY=VALUE → -e KEY=VALUE，紧接 extra_env 之后追加（与多机 build_common_docker_prefix 顺序一致：
    // extra_env → vllm_env → NCCL_DEBUG 默认值检查，模型显式设置的键会被跳过默认值注入）。
    // KEY 校验与 extra_env 同规则（全大写+数字+下划线 + 长度 > 1），非法键静默跳过。
    if let Some(env_list) = vllm_env.as_deref().filter(|e| !e.is_empty()) {
        let mut applied = Vec::new();
        for raw in env_list {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            if let Some((k, v)) = raw.split_once('=') {
                let k = k.trim();
                let v = v.trim();
                let valid_key = !k.is_empty()
                    && !v.is_empty()
                    && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                    && k.len() > 1;
                if valid_key {
                    extra_env_keys.insert(k.to_string());
                    extra_env_args.push("-e".to_string());
                    extra_env_args.push(format!("{}={}", k, v));
                    applied.push(format!("{}={}", k, v));
                }
            }
        }
        if !applied.is_empty() {
            app.emit(
                "model-log",
                serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[模型配置] 已应用模型清单 vllm_env（优先级最高）：{}", applied.join(", ")),
                    "source": "stdout",
                }),
            )
            .ok();
            crate::common::utils::logger::write_log("INFO", "MODEL", &format!("[{}] 应用模型清单 vllm_env: {}", model_id, applied.join(", ")));
        }
    }
    // NCCL_DEBUG 默认值：extra_env / vllm_env 均未显式设置时才注入（键集合已含模型清单键）
    if !extra_env_keys.contains("NCCL_DEBUG") {
        extra_env_args.push("-e".to_string());
        extra_env_args.push("NCCL_DEBUG=INFO".to_string());
    }

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
        "--ulimit".to_string(),
        "stack=67108864".to_string(),
        "--ipc".to_string(),
        "host".to_string(),
        "-p".to_string(),
        format!("{}:{}", port, port),
        "-v".to_string(),
        format!("{}:{}:ro", mount_src, mount_dst),
    ];
    // 注入额外环境变量（必须在 image 之前）
    args.extend(extra_env_args);
    // 清空镜像自带 ENTRYPOINT（如 vllm/vllm-openai 系的 ["vllm","serve"]）：
    // vllm_image 只是镜像名，启动命令完整自持（`vllm serve ...` 原样执行），
    // 绝不与镜像 ENTRYPOINT 拼接（否则命令会被 ENTRYPOINT 吞掉：单机变
    // `vllm serve vllm serve ...`，bash 型 ENTRYPOINT 报 `bash: line 1: vllm: command not found`）
    args.push("--entrypoint=".to_string());
    args.push(image);
    args.push("vllm".to_string());
    args.push("serve".to_string());
    args.push(mount_dst);
    args.push("--host".to_string());
    args.push(host);
    args.push("--port".to_string());
    args.push(port.to_string());

    // ===== 设置页 vLLM 详细参数（仅非空/非默认值才追加） =====
    // 统一调用 push_vllm_args（与多机 head 同源，避免遗漏新参数）
    push_vllm_args(&mut args, &vllm_args, params.ctx_size);

    // ===== 模型配置 vLLM 参数（vllm_flags）=====
    // 最后追加，优先级最高：同名参数可覆盖设置页配置与默认值。
    // 每条格式 "--key value" 或 "--flag"。
    if let Some(flags) = vllm_flags.as_deref().filter(|f| !f.is_empty()) {
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
                    "line": format!("[模型配置] 已应用模型清单 vllm_flags（优先级最高）：{}", applied.join(" ")),
                    "source": "stdout",
                }),
            )
            .ok();
            crate::common::utils::logger::write_log("INFO", "MODEL", &format!("[{}] 应用模型清单 vllm_flags: {}", model_id, applied.join(" ")));
        }
    }

    dbg_log!("vllm docker args: {:?}", args);

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
                    crate::common::utils::logger::write_log("INFO", "vLLM", &line);
                    app_c
                        .emit("model-log", serde_json::json!({
                            "model_id": &mid, "line": line.clone(), "source": "stdout",
                        }))
                        .ok();
                    // 推理引擎就绪信号
                    if line.contains("Application startup complete")
                        || line.contains("Starting vLLM API server")
                        || line.contains("Uvicorn running on")
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
                    crate::common::utils::logger::write_log("WARN", "vLLM", &line);
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
    vllm_image: Option<String>,
    vllm_flags: Option<Vec<String>>,
    vllm_env: Option<Vec<String>>,
) -> Result<(), AppError> {
    // 统一捕获启动失败并写入本地日志
    let result = start_model_inner(&app, &state, &model_id, params, device, vllm_image, vllm_flags, vllm_env).await;
    if let Err(ref e) = result {
        crate::common::utils::logger::write_log("ERROR", "MODEL", &format!("[{}] 启动失败: {}", model_id, e));
    }
    result
}

async fn start_model_inner(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
    model_id: &str,
    params: LaunchParams,
    device: Option<String>,
    vllm_image: Option<String>,
    vllm_flags: Option<Vec<String>>,
    vllm_env: Option<Vec<String>>,
) -> Result<(), AppError> {
    {
        let pid_lock = state.running_process.lock().map_err(|e| e.to_string())?;
        if pid_lock.is_some() {
            bail!("已有模型在运行中，请先停止当前模型");
        }
    }
    // 下载/镜像拉取进行中禁止启动：下载阶段「已下载」后后台仍在拉镜像，
    // 此时点击启动会与后台拉镜像并发执行 docker pull（同一镜像双 pull）。
    {
        let phases = state.downloading_phase.lock().map_err(|e| e.to_string())?;
        if phases.contains_key(model_id) {
            bail!("模型 {} 正在下载或拉取镜像，请稍候再试（完成后按钮自动可点）", model_id);
        }
    }

    let data_dir = config::get_data_dir(Some(app))?;
    let models_dir = data_dir.join("models");
    let model_dir = models_dir.join(model_id);

    // ===== 新格式（safetensors 目录模型）：vLLM Docker 启动 =====
    let is_dir_model = model_dir.join(".done").exists()
        || (model_dir.join("config.json").exists() && model_dir.join("model.safetensors").exists());
    if is_dir_model {
        // 多机互联：设置页启用且节点数 >= 2 时走集群启动（单机路径不变）
        let (mn, _) = load_multi_node_config(app);
        if mn.enabled && mn.nodes.len() >= 2 {
            return start_multi_node(app, state, model_id, &model_dir, params, vllm_image, vllm_flags, vllm_env).await;
        }
        return start_vllm_docker(app, state, model_id, &model_dir, params, device, vllm_image, vllm_flags, vllm_env).await;
    }

    // 仅支持 vLLM Docker 部署（safetensors 目录模型）
    Err(AppError::msg("当前仅支持推理引擎（safetensors 目录）模型，请下载新版模型后重试"))
}

#[tauri::command]
pub async fn stop_model(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    let pid = {
        let pid_lock = state.running_process.lock().map_err(|e| e.to_string())?;
        pid_lock.ok_or("没有正在运行的模型")?
    };
    // vLLM Docker 模式：先优雅停止容器，再兜底杀进程
    let container = state.running_container.lock().map_err(|e| e.to_string())?.clone();
    if let Some(container_name) = container {
        // 多机模式（容器名 adm-vllm-<model>-rank-0）：先 SSH 停止所有远端节点容器
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
            // 容器清理失败才兜底强杀 docker CLI（内部带进程名校验，防 pid 复用误杀会话进程）
            crate::common::utils::platform::kill_process_tree(pid);
        } else {
            crate::common::utils::logger::write_log(
                "INFO",
                "MODEL",
                &format!("[{}] 容器已成功停止，docker CLI 随 stop 退出，跳过进程树强杀", container_name),
            );
        }
    }
    // 非容器模式（异常残留状态）：仍按进程名校验兜底强杀
    if state.running_container.lock().map_err(|e| e.to_string())?.is_none() {
        crate::common::utils::platform::kill_process_tree(pid);
    }

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

/// 停止指定模型的下载：置位取消标志后，下载循环感知到即停止（保留 .part 供续传）。
/// 没有正在进行的下载时返回错误（前端据此清除幽灵下载状态）。
#[tauri::command]
pub async fn cancel_download(app: tauri::AppHandle, model_id: String) -> Result<(), AppError> {
    let flag = app
        .state::<AppState>()
        .download_cancel
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&model_id)
        .cloned();
    match flag {
        Some(flag) => {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
        None => Err(AppError::msg("没有正在进行的下载")),
    }
}
