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
                                // 跳过 hfd.sh 残留控件（.aria2）与点开头元数据（.done / .gitattributes 等）
                                if !name_str.starts_with('.') && !name_str.ends_with(".part") && !name_str.ends_with(".aria2") {
                                    files.push(name_str);
                                }
                            }
                        }
                    }
                }
                if !files.is_empty() {
                    let has_done = path.join(".done").exists();
                    models.push(LocalModel { model_id: dir_str, files, has_done });
                }
            }
        } else if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "gguf" {
                    if let Some(stem) = path.file_stem() {
                        let model_id = stem.to_string_lossy().to_string();
                        let filename = path.file_name().unwrap().to_string_lossy().to_string();
                        models.push(LocalModel { model_id, files: vec![filename], has_done: false });
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

/// 下载退出清理：移除 downloading_progress / downloading_phase / download_cancel 状态
/// （URL 清单与 hfd.sh 仓库下载两条路径共用，函数返回时统一触发）
struct DownloadCleanupGuard {
    h: tauri::AppHandle,
    id: String,
}

impl Drop for DownloadCleanupGuard {
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

/// `model_download_files` 条目是否为 HF 仓库 ID（形如 `org/name`，非 URL）：
/// 解析 HF 仓库条目：`org/name` 或 `org/name/<仓库内路径或通配模式>`（第 3 段起为 `--include` 模式）。
/// 完整文件 URL（http/https）返回 None（由逐文件 URL 下载路径处理）。
fn parse_repo_entry(entry: &str) -> Option<(String, Option<String>)> {
    let e = entry.trim();
    if e.is_empty() || e.starts_with("http://") || e.starts_with("https://") {
        return None;
    }
    let mut parts = e.splitn(3, '/');
    let org = parts.next().unwrap_or("").trim();
    let name = parts.next().unwrap_or("").trim();
    if org.is_empty()
        || name.is_empty()
        || org.contains(char::is_whitespace)
        || name.contains(char::is_whitespace)
    {
        return None;
    }
    let pattern = parts
        .next()
        .map(|p| p.trim().trim_start_matches('/').to_string())
        .filter(|p| !p.is_empty());
    Some((format!("{}/{}", org, name), pattern))
}

/// 条目是否为 HF 仓库条目（含带模式的 `org/name/path` 形式）。
fn is_repo_entry(entry: &str) -> bool {
    parse_repo_entry(entry).is_some()
}

/// 合并仓库条目：同一仓库的多条 `org/name/模式` 收敛为 (repo, [模式...])，
/// 保持首次出现顺序、模式去重；无模式的条目表示整仓下载。
fn merge_repo_entries(files: &[String]) -> Vec<(String, Vec<String>)> {
    let mut merged: Vec<(String, Vec<String>)> = Vec::new();
    for entry in files {
        let Some((repo, pattern)) = parse_repo_entry(entry) else { continue };
        match merged.iter_mut().find(|(r, _)| *r == repo) {
            Some((_, pats)) => {
                if let Some(p) = pattern {
                    if !pats.contains(&p) {
                        pats.push(p);
                    }
                }
            }
            None => merged.push((repo, pattern.into_iter().collect())),
        }
    }
    merged
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
        let expanded: Vec<String> = m
            .model_download_files
            .iter()
            .flat_map(|u| if is_repo_entry(u) { vec![u.clone()] } else { expand_shard_urls(u) })
            .collect();
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

    // ===== 下载清单分发（model_download_files 统一入口）=====
    // 1) 仓库条目（`org/name` 或 `org/name/路径或通配模式`）→ hfd.sh 下载（同仓库条目合并 --include）
    // 2) 完整文件 URL（http/https）→ 逐文件 .part 续传
    // 两种格式不可混用（与历史约束一致）
    if let Some(files) = model_files {
        let repo_count = files.iter().filter(|f| is_repo_entry(f)).count();
        if repo_count > 0 {
            if repo_count != files.len() {
                bail!(
                    "模型 {} 的 model_download_files 混用了仓库条目与文件 URL 格式，请检查远程 model.json 配置",
                    model_id
                );
            }
            let repos = merge_repo_entries(&files);
            if repos.is_empty() {
                bail!(
                    "模型 {} 的仓库条目解析失败，请检查 model.json（形如 org/name 或 org/name/目录/模式）",
                    model_id
                );
            }
            return hfd_download_repos(&app, &model_id, repos, vllm_image, cancel_flag).await;
        }
        {
            // 分片 URL 已在 fetch_model_list 中展开，此处直接使用
            let total = files.len();
            app.state::<AppState>().downloading_progress.lock().unwrap_or_else(|e| e.into_inner()).insert(model_id.clone(), 0u8);

            let _guard = DownloadCleanupGuard { h: app.clone(), id: model_id.clone() };

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

            // 全部文件下载完成：写 .done + 发完成事件 + 拉取 vLLM 镜像（与 hfd.sh 路径共用）
            finish_model_download(&app, &model_id, &model_dir, vllm_image).await;
            return Ok(());
        }
    }

    // model_url 旧格式单文件下载已废弃：新格式模型必须带 model_download_files
    bail!(
        "模型 {} 缺少下载文件清单（model_download_files 必填），请检查远程 model.json 配置",
        model_id
    );
}

/// 下载成功收尾（URL 清单与 hfd.sh 仓库下载两条路径共用）：
/// 写 .done 标记（scan_local_models 排除）→ 立刻发 download-complete(all)
/// （前端立即显示「已下载」+ 启动按钮可点）→ 进入镜像拉取阶段（downloading_phase 记
/// `<pull-image>`，start_model 据此拒绝并发启动，避免与后台拉镜并发 docker pull）
/// 并拉取 vLLM 镜像（镜像策略：远程 model.json 的 vllm_image 字段唯一指定）。
async fn finish_model_download(
    app: &tauri::AppHandle,
    model_id: &str,
    model_dir: &std::path::Path,
    vllm_image: Option<String>,
) {
    std::fs::write(model_dir.join(".done"), "").ok();
    app.emit(
        "download-complete",
        serde_json::json!({ "model_id": model_id, "type": "model", "all": true }),
    )
    .ok();
    if let Ok(mut map) = app.state::<AppState>().downloading_phase.lock() {
        map.insert(model_id.to_string(), "<pull-image>".to_string());
    }
    // 镜像名必须由后端重新拉取远程清单解析（与启动流程一致），不能依赖前端卡片透传——
    // 远程配置更新后旧卡片透传值为空，会导致镜像被静默跳过，最终启动时才报「镜像尚未下载」。
    // ComfyUI 镜像同样由 registry 拉取（应用内不再提供构建流程）。
    let resolved_image = resolve_engine_image(model_id, vllm_image.as_deref()).await;
    if let Some(image) = resolved_image {
        pull_image_if_configured(app, model_id, Some(&image)).await;
    } else {
        // 无法解析镜像名（远程清单拉取失败且无前端兜底，或清单缺 image 字段）：明确报错，不静默
        let msg = format!(
            "[ERROR] 模型 {} 无法解析镜像配置（远程 model.json 的 engine_image / vllm_image 必填，或远程清单拉取失败），镜像未拉取；请检查网络后重新点击「下载」触发",
            model_id
        );
        crate::common::utils::logger::write_log("ERROR", "DOWNLOAD", &msg);
        app.emit(
            "model-log",
            serde_json::json!({
                "model_id": model_id,
                "line": msg,
                "source": "stderr",
            }),
        )
        .ok();
        app.emit(
            "download-complete",
            serde_json::json!({
                "model_id": model_id,
                "type": "image-pull-failed",
                "image": "",
                "error": "无法解析镜像配置（远程 model.json 的 engine_image / vllm_image 必填，或远程清单拉取失败）",
            }),
        )
        .ok();
    }
}

/// HF 仓库条目下载（`model_download_files` 中的仓库条目，safetensors 目录模型）：
/// 条目形态 `org/name` 或 `org/name/<仓库内路径或通配模式>`（支持 `*`），
/// 同一仓库的多条条目合并为一次下载、模式去重后作为 hfd `--include`（无模式 = 整仓）；
/// 多个仓库按条目首次出现顺序依次下载。
/// 1. 确保 `<data>/hfd.sh` 工具就绪（首次从 hf-mirror 下载，chmod 755，全局复用）；
/// 2. 端点策略与 URL 清单一致：未配代理 → `HF_ENDPOINT=https://hf-mirror.com`；
///    配了代理 → hfd 默认源（huggingface.co）+ 注入代理 env；
/// 3. `bash hfd.sh <repo> --local-dir <models>/<id> [--include ...]`，输出转发 model-log
///    （wget/aria2 进度条行内含 `\r`，跳过避免刷屏）；
/// 4. 每 2s 轮询模型目录 + HF API 文件清单（同样按模式过滤）折算进度/速度；
/// 5. 取消：cancel_flag 置位 → 杀整个进程组（bash + aria2c/wget），发 download-cancelled；
/// 6. 全部成功：finish_model_download 写 .done + 发完成事件 + 拉取镜像（与 URL 清单路径共用）。
async fn hfd_download_repos(
    app: &tauri::AppHandle,
    model_id: &str,
    repos: Vec<(String, Vec<String>)>,
    vllm_image: Option<String>,
    cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), AppError> {
    let _guard = DownloadCleanupGuard { h: app.clone(), id: model_id.to_string() };
    let script = ensure_hfd_script(app, model_id).await?;
    ensure_aria2c(app, model_id).await?;
    let data_dir = config::get_data_dir(Some(app))?;
    let model_dir = data_dir.join("models").join(model_id);
    let first_label = repos.first().map(|(r, _)| r.clone()).unwrap_or_default();

    app.state::<AppState>()
        .downloading_progress
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(model_id.to_string(), 0u8);
    if let Ok(mut map) = app.state::<AppState>().downloading_phase.lock() {
        map.insert(model_id.to_string(), first_label.clone());
    }
    app.emit(
        "download-progress",
        serde_json::json!({ "model_id": model_id, "progress": 0u8, "file": &first_label, "type": "model" }),
    )
    .ok();

    // 端点策略与 URL 清单一致：未配代理 → hf-mirror；配代理 → hfd 默认源 + 代理 env
    let proxy = crate::common::utils::proxy::proxy_url(app).await;
    let endpoint = if proxy.is_empty() {
        "https://hf-mirror.com".to_string()
    } else {
        String::new()
    };

    for (repo, includes) in &repos {
        if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            app.emit(
                "download-cancelled",
                serde_json::json!({ "model_id": model_id, "type": "model" }),
            )
            .ok();
            return Ok(());
        }
        crate::common::utils::logger::write_log(
            "INFO",
            "DOWNLOAD",
            &format!(
                "[{}] hfd 下载仓库 {}（include 模式 {} 条{}）",
                model_id,
                repo,
                includes.len(),
                if includes.is_empty() { "，整仓" } else { "" }
            ),
        );
        match hfd_download_one(app, model_id, repo, includes, &script, &model_dir, &endpoint, &proxy, &cancel_flag).await? {
            HfdOutcome::Cancelled => return Ok(()),
            HfdOutcome::Exited(st) => {
                if !st.success() {
                    return Err(AppError::msg(format!(
                        "hfd.sh 下载失败（仓库 {}，退出码 {:?}），详见模型日志",
                        repo,
                        st.code()
                    )));
                }
            }
        }
    }

    finish_model_download(app, model_id, &model_dir, vllm_image).await;
    Ok(())
}

/// 单个仓库的 hfd 执行结果
enum HfdOutcome {
    /// 子进程已结束（退出码由调用方判断）
    Exited(std::process::ExitStatus),
    /// 用户取消（已杀进程组并发出 download-cancelled）
    Cancelled,
}

/// 单个仓库的 hfd 执行 + 日志转发 + 进度折算（由 hfd_download_repos 逐仓库调用）。
/// `endpoint` 为空 = hfd 默认源（配代理场景），否则注入 `HF_ENDPOINT`（hf-mirror）。
#[allow(clippy::too_many_arguments)]
async fn hfd_download_one(
    app: &tauri::AppHandle,
    model_id: &str,
    repo: &str,
    includes: &[String],
    script: &std::path::Path,
    model_dir: &std::path::Path,
    endpoint: &str,
    proxy: &str,
    cancel_flag: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<HfdOutcome, AppError> {
    use tokio::io::AsyncBufReadExt;

    // 进度基准：本仓库文件清单（含大小），按 include 模式过滤；
    // 拉取失败时在轮询循环里退回 hfd 自带清单（`.hfd/manifest`），避免进度停在 0%
    let api_endpoint = if endpoint.is_empty() { "https://huggingface.co" } else { endpoint };
    let http = crate::common::utils::proxy::build_download_http(
        app,
        Some(std::time::Duration::from_secs(60)),
    )
    .await?;
    let mut repo_files = fetch_repo_files(&http.client, api_endpoint, repo)
        .await
        .map(|files| {
            files
                .into_iter()
                .filter(|(name, _)| matches_include_patterns(name, includes))
                .collect::<Vec<_>>()
        });

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg(script).arg(repo).arg("--local-dir").arg(model_dir);
    if !includes.is_empty() {
        cmd.arg("--include");
        for pat in includes {
            cmd.arg(pat);
        }
        crate::common::utils::logger::write_log(
            "INFO",
            "DOWNLOAD",
            &format!("[{}] hfd include 过滤: {}", model_id, includes.join(" ")),
        );
    }
    if !endpoint.is_empty() {
        cmd.env("HF_ENDPOINT", endpoint);
    }
    if !proxy.is_empty() {
        cmd.env("http_proxy", proxy).env("https_proxy", proxy).env("all_proxy", proxy);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // 早退（如等待异常）时杀掉子进程，防孤儿 hfd.sh 继续写文件
    cmd.kill_on_drop(true);
    // 进程组隔离：pgid == bash pid，取消时 kill -<pgid> 可连带终止 aria2c/wget 子进程
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::msg(format!("启动 hfd.sh 失败: {}", e)))?;
    let pid = child.id().unwrap_or(0);

    for is_err in [false, true] {
        let stream = if is_err {
            child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)
        } else {
            child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)
        };
        let Some(stream) = stream else { continue };
        let app_c = app.clone();
        let mid = model_id.to_string();
        let source = if is_err { "stderr" } else { "stdout" };
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // wget/aria2 进度条以 \r 刷新同一行：按 \r 切段，丢弃含 % 的进度段，
                // 保留普通消息/报错段（报错常嵌在 \r 流里，不能整行丢弃）
                for seg in line.split('\r') {
                    let seg = seg.trim();
                    if seg.is_empty() || seg.contains('%') {
                        continue;
                    }
                    let truncated: String = seg.chars().take(400).collect();
                    // 后端直接落盘（不依赖前端事件转发），保证日志文件始终有 hfd 输出
                    crate::common::utils::logger::write_log(
                        if is_err { "WARN" } else { "INFO" },
                        "DOWNLOAD",
                        &format!("[{}] [hfd] {}", mid, truncated),
                    );
                    app_c
                        .emit(
                            "model-log",
                            serde_json::json!({ "model_id": mid, "line": truncated, "source": source }),
                        )
                        .ok();
                }
            }
        });
    }

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = tx.send(child.wait().await);
    });

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(2));
    ticker.tick().await;

    let mut tracker = SpeedTracker::from_existing(0);
    let mut last_progress = 0u8;
    let mut last_speed = 0u64;
    let status = loop {
        tokio::select! {
            st = &mut rx => {
                break st.map_err(|e| AppError::msg(format!("等待 hfd.sh 退出失败: {}", e)))?;
            }
            _ = ticker.tick() => {
                if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    kill_hfd_process_group(pid, false);
                    // TERM 后留宽限期再 KILL：确保 aria2c/wget 不残留（防孤儿进程与重下双写竞争）
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    kill_hfd_process_group(pid, true);
                    app.emit(
                        "download-cancelled",
                        serde_json::json!({ "model_id": model_id, "type": "model" }),
                    )
                    .ok();
                    return Ok(HfdOutcome::Cancelled);
                }
                // 基准拉取失败时退回 hfd 自带清单（hfd 列完文件后写 `<local-dir>/.hfd/manifest`，
                // 内容同样按 --include 过滤）：缺基准会让整段下载的进度都停在 0%
                if repo_files.is_none() {
                    repo_files = read_hfd_manifest(model_dir);
                    if repo_files.is_some() {
                        crate::common::utils::logger::write_log(
                            "INFO",
                            "DOWNLOAD",
                            &format!("[{}] 进度基准改用 hfd 清单（HF API 不可达）", model_id),
                        );
                    }
                }
                if let Some(files) = &repo_files {
                    let (overall, bytes, cur_file) = hfd_poll_progress(model_dir, files);
                    let speed = tracker.update(bytes);
                    if let Some(f) = &cur_file {
                        if let Ok(mut map) = app.state::<AppState>().downloading_phase.lock() {
                            map.insert(model_id.to_string(), f.clone());
                        }
                    }
                    if overall != last_progress || speed != last_speed {
                        last_progress = overall;
                        last_speed = speed;
                        if let Ok(mut map) = app.state::<AppState>().downloading_progress.lock() {
                            map.insert(model_id.to_string(), overall);
                        }
                        app.emit(
                            "download-progress",
                            serde_json::json!({
                                "model_id": model_id,
                                "progress": overall,
                                "speed": speed,
                                "file": cur_file.unwrap_or_default(),
                                "type": "model",
                            }),
                        )
                        .ok();
                    }
                }
            }
        }
    };

    let status = status.map_err(|e| AppError::msg(format!("hfd.sh 执行失败: {}", e)))?;
    Ok(HfdOutcome::Exited(status))
}


/// 确保 aria2c 可用（hfd.sh 优先走 aria2c 多连接断点续传）。未安装时自动安装：
/// 免密 `sudo -n` → 失败则 `pkexec` 弹系统密码框提权（对齐 settings.rs 镜像配置链路）。
/// 两种方式都失败：本机有 wget → 警告后降级 wget 模式继续（hfd 自动回退）；
/// wget 也缺 → 报错并附手动安装命令。
async fn ensure_aria2c(app: &tauri::AppHandle, model_id: &str) -> Result<(), AppError> {
    let log = |line: &str, source: &str| {
        crate::common::utils::logger::write_log(
            if source == "stderr" { "WARN" } else { "INFO" },
            "DOWNLOAD",
            &format!("[{}] [hfd] {}", model_id, line),
        );
        app.emit(
            "model-log",
            serde_json::json!({ "model_id": model_id, "line": line, "source": source }),
        )
        .ok();
    };
    let have = |cmd: &str| {
        let cmd = cmd.to_string();
        async move {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("command -v {} >/dev/null 2>&1", cmd))
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false)
        }
    };

    if have("aria2c").await {
        return Ok(());
    }
    log("检测到 aria2c 未安装，正在自动安装（需要系统密码）...", "stdout");

    // 1) 免密 sudo；apt 索引缺失时先 update 再装
    let install_cmd = "apt-get install -y aria2 || (apt-get update -y && apt-get install -y aria2)";
    let sudo_ok = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "sudo -n sh -c {} 2>&1",
            crate::common::ssh::sh_quote(install_cmd)
        ))
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    // 2) 免密失败 → pkexec 弹窗提权（用户取消/未授权则视为失败）
    if !sudo_ok {
        log("免密 sudo 不可用，尝试通过系统授权窗口安装...", "stdout");
        let out = tokio::process::Command::new("pkexec")
            .args(["sh", "-c", install_cmd])
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                if stderr.contains("Not authorized")
                    || stderr.contains("dismissed")
                    || stderr.contains("cancel")
                {
                    log("系统授权窗口已取消，aria2c 未安装", "stderr");
                } else {
                    log(format!("aria2c 自动安装失败: {}", stderr.trim()).as_str(), "stderr");
                }
            }
            Err(e) => log(format!("启动 pkexec 失败: {}", e).as_str(), "stderr"),
        }
    }

    if have("aria2c").await {
        log("aria2c 安装完成", "stdout");
        return Ok(());
    }
    if have("wget").await {
        log("无法安装 aria2c，将使用 wget 模式下载（不支持多连接，速度较慢）", "stderr");
        return Ok(());
    }
    bail!(
        "aria2c 与 wget 均未安装且自动安装失败；请手动执行：sudo apt-get update && sudo apt-get install -y aria2，然后重新点击下载"
    );
}

/// hfd.sh 工具下载地址（hf-mirror 官方分发）
const HFD_SCRIPT_URL: &str = "https://hf-mirror.com/hfd/hfd.sh";

/// 确保 hfd.sh 工具就绪：`<data>/hfd.sh` 不存在时从 hf-mirror 下载（代理感知）并 chmod 755。
/// 工具全局复用，所有模型的整仓下载共用同一份脚本。
async fn ensure_hfd_script(app: &tauri::AppHandle, model_id: &str) -> Result<std::path::PathBuf, AppError> {
    let data_dir = config::get_data_dir(Some(app))?;
    let script = data_dir.join("hfd.sh");
    if script.exists() {
        return Ok(script);
    }
    let http = crate::common::utils::proxy::build_download_http(
        app,
        Some(std::time::Duration::from_secs(60)),
    )
    .await?;
    app.emit(
        "model-log",
        serde_json::json!({
            "model_id": model_id,
            "line": "[hfd] 首次使用，正在下载 hfd.sh 工具...",
            "source": "stdout",
        }),
    )
    .ok();
    let resp = http
        .client
        .get(HFD_SCRIPT_URL)
        .send()
        .await
        .map_err(|e| AppError::msg(format!("下载 hfd.sh 失败: {}", e)))?;
    if !resp.status().is_success() {
        bail!("下载 hfd.sh 失败: 服务器返回 {}", resp.status());
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::msg(format!("读取 hfd.sh 内容失败: {}", e)))?;
    std::fs::write(&script, &bytes).map_err(|e| AppError::msg(format!("写入 hfd.sh 失败: {}", e)))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755));
    }
    Ok(script)
}

/// 拉取 HF 仓库文件清单（`/api/models/<repo>?blobs=true`，含文件大小）作为进度基准。
/// 点开头元数据（.gitattributes 等）不计入。失败返回 None（进度退化为不更新，仅日志跟随）。
async fn fetch_repo_files(
    client: &reqwest::Client,
    endpoint: &str,
    repo: &str,
) -> Option<Vec<(String, u64)>> {
    let url = format!("{}/api/models/{}?blobs=true", endpoint.trim_end_matches('/'), repo);
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    let siblings = v.get("siblings")?.as_array()?;
    Some(
        siblings
            .iter()
            .filter_map(|s| {
                let name = s.get("rfilename")?.as_str()?.to_string();
                if name.starts_with('.') {
                    return None;
                }
                let size = s.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
                Some((name, size))
            })
            .collect(),
    )
}

/// 读取 hfd.sh 自己的下载清单（`<local-dir>/.hfd/manifest`，行格式 `大小\t相对路径`）
/// 作为进度基准的兜底：内容与 hfd 实际下载的文件一致（已按 --include 过滤），
/// HF API 不可达或首次运行尚未生成清单时返回 None。
fn read_hfd_manifest(model_dir: &std::path::Path) -> Option<Vec<(String, u64)>> {
    let text = std::fs::read_to_string(model_dir.join(".hfd").join("manifest")).ok()?;
    let mut out: Vec<(String, u64)> = Vec::new();
    for line in text.lines() {
        let Some((size, path)) = line.trim_end().split_once('\t') else { continue };
        let path = path.trim().trim_start_matches("./").to_string();
        if path.is_empty() || path.starts_with('.') {
            continue;
        }
        out.push((path, size.trim().parse::<u64>().unwrap_or(0)));
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 轮询模型目录折算下载进度（递归扫 depth ≤ 3 子目录，仓库含文件夹时进度仍可折算）：
/// 按**字节**折算——仓库内文件大小差异极大（如 H3 = 两个 21 GB DiT + 15.7 GB 编码器 + 若干小文件），
/// 按文件数折算会被大文件拖成「长时间停在 0%」，断点续传后更是显示 0% 不动。
/// 已完成 = 文件在且无对应 `.aria2` 控件且（大小未知或已到齐）→ 计预期全量；
/// 进行中的文件（含续传留下的部分文件）计已落盘字节（见 hfd_scan_dir）。
/// 返回 (总进度百分比, 已落盘字节, 进行中文件名)。
fn hfd_poll_progress(model_dir: &std::path::Path, files: &[(String, u64)]) -> (u8, u64, Option<String>) {
    let mut present: Vec<(String, u64, u64)> = Vec::new();
    hfd_scan_dir(model_dir, "", 3, &mut present);
    let disk: std::collections::HashMap<&str, (u64, u64)> =
        present.iter().map(|(n, l, a)| (n.as_str(), (*l, *a))).collect();
    let total_known: u64 = files.iter().map(|(_, s)| *s).sum();
    let mut done = 0usize;
    let mut downloaded = 0u64;
    let mut frac = 0f32;
    let mut cur: Option<(String, u64)> = None;
    for (name, size) in files {
        let Some((len, allocated)) = disk.get(name.as_str()).copied() else { continue };
        // aria2 断点续传时文件带 `.aria2` 控件：即使长度已到齐也需以控件消失为完成标志
        let aria2 = model_dir.join(format!("{}.aria2", name)).exists();
        let is_done = !aria2 && (*size == 0 || len >= *size);
        let written = if is_done {
            if *size > 0 {
                *size
            } else {
                len
            }
        } else if *size > 0 {
            // 已分配块数（稀疏感知）即真实已落盘字节，按预期大小裁剪
            allocated.min(*size)
        } else {
            allocated
        };
        downloaded += written;
        if is_done {
            done += 1;
            continue;
        }
        if *size > 0 {
            frac += (written as f32 / *size as f32).min(1.0);
        }
        if cur.as_ref().map(|(_, cw)| written > *cw).unwrap_or(true) {
            cur = Some((name.clone(), written));
        }
    }
    let overall = if total_known > 0 {
        ((downloaded as f64 * 100.0 / total_known as f64).min(99.0)) as u8
    } else {
        // 清单未带大小（API 未返回 size）→ 退回按文件数折算
        (((done as f32 + frac) * 100.0 / files.len().max(1) as f32).min(99.0)) as u8
    };
    (overall, downloaded, cur.map(|c| c.0))
}

/// 递归收集 dir 下相对路径文件 → (相对路径, 文件长度, 已落盘字节估计)
/// （跳过点开头条目：`.done` / `.aria2` 控件 / hfd 的 `.hfd` 状态目录）。
/// 已落盘字节用于折算 aria2 多段并行下载的真实进度：`--file-allocation=none` 下文件按段
/// 稀疏写入，文件长度会在开始后瞬间逼近全量（最后一段立刻开始写），只有已分配的数据块
/// 代表真实进度；其它平台退回文件长度。
fn hfd_scan_dir(
    dir: &std::path::Path,
    prefix: &str,
    depth: usize,
    out: &mut Vec<(String, u64, u64)>,
) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let rel = if prefix.is_empty() { name } else { format!("{}/{}", prefix, name) };
        let p = e.path();
        if p.is_dir() {
            hfd_scan_dir(&p, &rel, depth - 1, out);
        } else if let Ok(m) = e.metadata() {
            out.push((rel, m.len(), crate::common::utils::download::file_allocated_bytes(&m)));
        }
    }
}

/// 极简 glob：仅支持 `*`（匹配任意字符序列，含 `/`），其余字符按字面比较。
/// 用于 hfd `--include` 模式与仓库文件相对路径匹配（`FL2VA/*`、`model_index.json` 等）。
fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !text[pos..].starts_with(part) {
                return false;
            }
            pos += part.len();
        } else if i == parts.len() - 1 {
            return text[pos..].ends_with(part);
        } else {
            match text[pos..].find(part) {
                Some(idx) => pos += idx + part.len(),
                None => return false,
            }
        }
    }
    true
}

/// 仓库文件相对路径是否命中仓库条目的 include 模式（条目第 3 段起的路径/通配，空列表 = 全部命中）。
/// hfd.sh 将模式转成正则后用“子串”匹配（`=~`），这里同样给模式补上隐式前后通配，
/// 保证进度基准与 hfd 的实际过滤一致（否则进度可能到不了 100%）。
fn matches_include_patterns(path: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| {
        let p = p.trim();
        if p.is_empty() {
            return false;
        }
        glob_match(&format!("*{}*", p), path)
    })
}

/// 终止 hfd.sh 进程组（process_group(0) → pgid == bash pid，aria2c/wget 同组一并终止）。
/// force=false 发 TERM，true 发 KILL；pid==0 时不动（kill -0 会命中调用方进程组，绝对禁止）。
fn kill_hfd_process_group(pid: u32, force: bool) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    {
        let sig = if force { "-KILL" } else { "-TERM" };
        let _ = std::process::Command::new("kill")
            .arg(sig)
            .arg(format!("-{}", pid))
            .status();
    }
    #[cfg(not(unix))]
    {
        let _ = force;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }
}

/// 从远程 model.json 解析指定模型的权威镜像（与启动流程一致，镜像由远程唯一指定）：
/// 优先 `engine_image`（当前引擎专用），为空回退旧字段 `vllm_image`。
/// 远程拉取失败时回退到前端透传的 fallback；两者都没有返回 None。
async fn resolve_engine_image(model_id: &str, fallback: Option<&str>) -> Option<String> {
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
            .and_then(|m| {
                let engine_image = m.engine_image.trim();
                let vllm_image = m.vllm_image.trim();
                if !engine_image.is_empty() {
                    Some(engine_image.to_string())
                } else if !vllm_image.is_empty() {
                    Some(vllm_image.to_string())
                } else {
                    None
                }
            })
            .or_else(from_fallback),
        Err(_) => from_fallback(),
    }
}

/// HTTP 就绪探活（模型清单 `engine_ready_probe`）：扩散/视频类服务（SGLang diffusion）
/// 没有 LLM 的固定就绪 banner，以 `GET /health` 返回 200 作为「可接受请求」信号
/// （SGLang 扩散服务 warmup 未完成时返回 503）。5s 轮询，最长 90 分钟（大模型冷加载可达
/// 数十分钟）；就绪后发一次 model-started 并退出，容器退出（run 状态清零）后自动停挑。
fn spawn_ready_probe(app: tauri::AppHandle, model_id: String, port: u16, ready_path: String) {
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        // 就绪路径：默认 /health（SGLang/vLLM），ComfyUI 等应用服务用 /system_stats
        let path = {
            let p = ready_path.trim();
            if p.starts_with('/') { p.to_string() } else { "/health".to_string() }
        };
        let url = format!("http://127.0.0.1:{}{}", port, path);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90 * 60);
        crate::common::utils::logger::write_log(
            "INFO",
            "MODEL",
            &format!("[{}] 已启用 HTTP 就绪探活: {}（返回 200 即就绪）", model_id, url),
        );
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            // 容器退出（start_*_docker 的退出清理会把 running_model_id 置空）→ 结束探活
            let running_id = app
                .state::<AppState>()
                .running_model_id
                .lock()
                .map(|g| g.clone())
                .unwrap_or(None);
            if running_id.as_deref() != Some(model_id.as_str())
                || std::time::Instant::now() > deadline
            {
                return;
            }
            let ready = matches!(client.get(&url).send().await, Ok(resp) if resp.status().is_success());
            if ready {
                crate::common::utils::logger::write_log(
                    "INFO",
                    "MODEL",
                    &format!("[{}] 就绪探活通过（/health 200）", model_id),
                );
                app.emit("model-log", serde_json::json!({
                    "model_id": model_id,
                    "line": "就绪探活通过（/health 返回 200），推理服务已可接受请求",
                    "source": "stdout",
                })).ok();
                app.emit("model-started", serde_json::json!({
                    "model_id": model_id,
                    "port": port,
                })).ok();
                return;
            }
        }
    });
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

/// 拉取镜像（`download_model` 在模型文件下完后调用，或「视频生成」页「下载镜像」直接调用）：
/// CLI/daemon/GPU 预检 → 镜像存在则跳过 → 否则 `docker pull`（多源回退/超时见 pull_image）。
/// 成功返回实际可用镜像名（可能是镜像源前缀版本），拉取失败返回错误。
pub(crate) async fn pull_docker_image(
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
pub(crate) async fn docker_preflight(app: &tauri::AppHandle, model_id: &str) -> Result<(), AppError> {
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
/// 2. 从 dangling 镜像中选择 RepoDigests 归属与目标 repo 一致的（多个取创建时间最新）
///    重新 `docker tag`——严格要求归属匹配，不匹配即报错（曾因“唯一 dangling 直接
///    tag”启发式把无关 sglang 旧镜像误打为 vllm 镜像名）；
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

    // 3. 兜底：dangling（untagged）镜像打 tag。
    //    仅当 RepoDigests 归属匹配目标 repo（如 `ghcr.io/xxx/yyy@sha256:`）时才修正，
    //    多个候选取创建时间最新。**绝不**对“唯一 dangling”直接 tag——历史上该启发式
    //    把无关 sglang 旧镜像误 tag 成目标镜像名（运行错误引擎）；pull 失败时本地
    //    恰好有旧 dangling 的场景不少见，宁可报错提示手动处理。
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
    let mut dangling: Vec<(String, String)> = Vec::new(); // (CreatedAt, ID)
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some((ts, id)) = line.split_once('|') {
            let ts = ts.trim().to_string();
            let id = id.trim().to_string();
            if !ts.is_empty() && !id.is_empty() {
                dangling.push((ts, id));
            }
        }
    }
    if dangling.is_empty() {
        return Err(AppError::msg(format!(
            "镜像 {} 拉取后未在本地找到镜像（含 untagged 兜底），请手动执行 docker pull {}",
            image, image
        )));
    }

    // 归属校验：untagged 镜像的 RepoDigests 必须包含目标 repo（如 `ghcr.io/xxx/yyy@sha256:`），
    // 否则是无关的历史 dangling 镜像（如其它引擎/旧版本残留），拒绝打 tag（宁缺毋滥）。
    let repo = split_image_repo_tag(image).0;
    let mut matched: Vec<(String, String)> = Vec::new();
    for (ts, id) in &dangling {
        let inspect = crate::common::utils::platform::docker_cmd_tokio()
            .args(["image", "inspect", "--format", "{{.RepoDigests}}", id])
            .output()
            .await;
        match inspect {
            Ok(o) if o.status.success() => {
                let digests = String::from_utf8_lossy(&o.stdout);
                if digests.contains(&format!("{}@sha256:", repo)) {
                    matched.push((ts.clone(), id.clone()));
                }
            }
            _ => {} // inspect 失败视为不匹配（继续遍历）
        }
    }
    let id = if !matched.is_empty() {
        matched.sort_by(|a, b| b.0.cmp(&a.0));
        matched[0].1.clone()
    } else {
        return Err(AppError::msg(format!(
            "镜像 {} 拉取后 tag 校验失败：本地 untagged 镜像均不属于 {}（如 {}），拒绝自动修正以免误 tag；请手动执行 docker pull {}，并在确认镜像内容无误后手动 docker tag",
            image,
            repo,
            dangling.iter().map(|(_, i)| i.as_str()).collect::<Vec<_>>().join(" / "),
            image
        )));
    };

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
    // 4. 最终确认
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
        bail!("多机 master 端口不能为 0");
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

/// 解析模型清单 `vllm_extra_mounts`（附加挂载清单）：
/// 1. 条目为 `host:container` 显式映射（容器路径以 `/` 开头）→ 按原样挂载：host 为绝对路径或
///    相对数据目录（`<data>`）的相对路径，目录不存在自动创建；如 H3 的媒体目录 `media:/data/minimax-h3`；
/// 2. 其余条目视为 model_id：对应本地 `<models>/<id>` 目录，校验下载完成
///    （.done 或 config.json+model.safetensors），未完成直接报错提示先在模型列表下载。
/// 返回 (宿主机路径, 容器路径) 列表，由调用方逐条转成 `-v <host>:<container>:ro`。
/// 空列表返回空——所有既有模型 docker 参数不变。
/// 解析模型清单 `vllm_extra_mounts`（附加挂载清单）：
/// 1. 条目为 `host:container` 显式映射（容器路径以 `/` 开头）→ 按原样挂载：host 为绝对路径或
///    相对数据目录（`<data>`）的相对路径，目录不存在自动创建；可再跟 `:ro` / `:rw` 覆盖默认模式
///    （如 H3 的媒体目录 `media:/data/minimax-h3:ro`、ComfyUI 的可写权重目录 `models/MiniMax-H3-ComfyUI:/opt/ComfyUI/models`）；
/// 2. 其余条目视为 model_id：对应本地 `<models>/<id>` 目录，校验下载完成
///    （.done 或 config.json+model.safetensors），未完成直接报错提示先在模型列表下载。
/// 返回 (宿主机路径, 容器路径, 是否只读) 列表，由调用方逐条转成 `-v <host>:<container>[:ro]`。
/// 空列表返回空——所有既有模型 docker 参数不变。
fn resolve_extra_mounts(
    models_dir: &std::path::Path,
    extra_mounts: &[String],
    default_read_only: bool,
) -> Result<Vec<(String, String, bool)>, AppError> {
    let data_dir = models_dir.parent().unwrap_or(models_dir).to_path_buf();
    let mut out = Vec::new();
    for entry in extra_mounts {
        let mut entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // 显式模式后缀（`:ro` / `:rw`）优先于调用方默认
        let mut read_only = default_read_only;
        if let Some(stripped) = entry
            .strip_suffix(":ro")
            .or_else(|| entry.strip_suffix(":rw"))
        {
            read_only = entry.ends_with(":ro");
            entry = stripped;
        }
        // 显式映射 `host:container`（容器路径必须以 / 开头，避免 Windows 盘符误判）
        if let Some((host_raw, container_raw)) = entry.rsplit_once(':') {
            let container = container_raw.trim();
            let host_raw = host_raw.trim();
            if container.starts_with('/') && !host_raw.is_empty() {
                let host_path = {
                    let raw = std::path::PathBuf::from(host_raw);
                    if raw.is_absolute() { raw } else { data_dir.join(raw) }
                };
                std::fs::create_dir_all(&host_path).map_err(|e| {
                    format!("创建附加挂载目录失败 {}: {}", host_path.display(), e)
                })?;
                out.push((
                    host_path.to_string_lossy().to_string(),
                    container.to_string(),
                    read_only,
                ));
                continue;
            }
        }
        let id = entry;
        let dir = models_dir.join(id);
        let complete = dir.join(".done").exists()
            || (dir.join("config.json").exists() && dir.join("model.safetensors").exists());
        if !complete {
            bail!("附加模型 {} 未下载或下载未完成，请先在模型列表下载该模型", id);
        }
        out.push((
            dir.to_string_lossy().to_string(),
            format!("/models/{}", id),
            read_only,
        ));
    }
    Ok(out)
}

/// 多机 worker 远端附加模型目录探活（vllm_extra_mounts）：与主模型同规则解析远端路径
/// `<model_root>/<id>`，检查下载完成标记；缺失时报错提示先下载/同步到该节点。
async fn probe_remote_extra_mounts(
    node: &crate::common::types::NodeInfo,
    key: Option<&str>,
    extra_mounts: &[String],
) -> Result<Vec<(String, String, bool)>, AppError> {
    let mut out = Vec::new();
    for id in extra_mounts {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        let dir = effective_remote_model_dir(&node.model_dir, &node.ssh_user, id);
        let script = format!(
            "d={dir_q}; if [ -f \"$d/.done\" ] || {{ [ -f \"$d/config.json\" ] && [ -f \"$d/model.safetensors\" ]; }}; then echo EXTRA_OK; else echo EXTRA_MISSING; fi",
            dir_q = crate::common::ssh::quote_remote_path(&dir),
        );
        let ok = crate::common::ssh::ssh_run(
            &node.ip, &node.ssh_user, node.ssh_port, key, &script,
            std::time::Duration::from_secs(10),
        )
        .await
        .map(|(_, o, _)| o.contains("EXTRA_OK"))
        .unwrap_or(false);
        if !ok {
            bail!("远端节点 {} 缺少附加模型 {}（目录 {}），请先在该节点下载或使用设置页「同步模型到直连节点」", node.ip, id, dir);
        }
        out.push((dir, format!("/models/{}", id), true));
    }
    Ok(out)
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

/// 拼装容器 `-e KEY=VALUE` 参数：设置页 extra_env → 模型清单 vllm_env（同名键后者生效，
/// 模型清单优先级最高）→ NCCL_DEBUG=INFO 默认值（两者均未显式设置时）。
/// KEY 全大写+数字+下划线且长度 > 1 才视为合法（避免空行/残行注入），非法键静默跳过。
/// vLLM / SGLang 单机启动共用。
fn build_container_env_args(
    app: &tauri::AppHandle,
    model_id: &str,
    settings_extra_env: &str,
    model_env: &[String],
) -> Vec<String> {
    let mut extra_env_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut extra_env_args: Vec<String> = Vec::new();
    for line in settings_extra_env.lines() {
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
    let mut applied = Vec::new();
    for raw in model_env {
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
    if !extra_env_keys.contains("NCCL_DEBUG") {
        extra_env_args.push("-e".to_string());
        extra_env_args.push("NCCL_DEBUG=INFO".to_string());
    }
    extra_env_args
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

/// 把设置页 vllm_args 中的通用子集映射为 SGLang `launch_server` 参数：
/// ctx_size → `--context-length`、tp → `--tp`、gpu-memory-utilization → `--mem-fraction-static`、
/// trust-remote-code → `--trust-remote-code`。其余 vLLM 专属字段（quantization / kv-cache /
/// load-format / block-size / parser / max-num-* 等）合法值两边不同，不映射，
/// 由模型清单 vllm_flags 承载完整 SGLang 配方（追加在最后，优先级最高）。
fn push_sglang_args(args: &mut Vec<String>, vllm_args: &VllmArgs, ctx_size: Option<i32>) {
    if let Some(ctx) = ctx_size {
        if ctx > 0 {
            args.extend(["--context-length".to_string(), ctx.to_string()]);
        }
    }
    if vllm_args.tensor_parallel_size > 1 {
        args.extend(["--tp".to_string(), vllm_args.tensor_parallel_size.to_string()]);
    }
    if vllm_args.gpu_memory_utilization > 0.0 {
        args.extend(["--mem-fraction-static".to_string(), format!("{}", vllm_args.gpu_memory_utilization)]);
    }
    if vllm_args.trust_remote_code {
        args.push("--trust-remote-code".to_string());
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
    container_name: &str,
    model_dir: &std::path::Path,
    image: &str,
    shm_size: &str,
    mn: &MultiNodeArgs,
    node_ip: &str,
    iface: Option<&str>,
    ib_iface: Option<&str>,
    has_infiniband: bool,
    model_env: &[String],
    extra_mounts: &[(String, String, bool)],
) -> Vec<String> {
    let mount_dst = format!("/models/{}", model_id);
    let mut args = build_common_docker_prefix(container_name, shm_size, node_ip, iface, ib_iface, has_infiniband, &mn.extra_env, model_env);
    args.push("-v".to_string());
    args.push(format!("{}:{}:ro", model_dir.to_string_lossy(), mount_dst));
    // vllm_extra_mounts：附加模型目录（drafter 等），主挂载之后逐条追加，空列表零变化
    for (host_path, container_path, read_only) in extra_mounts {
        args.push("-v".to_string());
        let mode = if *read_only { ":ro" } else { "" };
        args.push(format!("{}:{}{}", host_path, container_path, mode));
    }
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

/// 多机 head（rank 0）SGLang `launch_server` 命令参数：末尾追加 `--nnodes N --node-rank 0
/// --tp N --dist-init-addr <head_ip>:<port>`（各节点同一命令、仅 node-rank 不同；rank 0 对外服务，
/// TP 固定等于节点数，与 vLLM 多机路径语义一致）。
fn build_head_sglang_exec_args(
    model_id: &str,
    port: u16,
    vllm_args: &VllmArgs,
    vllm_flags: &Option<Vec<String>>,
    mn: &MultiNodeArgs,
    ctx_size: Option<i32>,
) -> Vec<String> {
    let mount_dst = format!("/models/{}", model_id);
    let mut args = vec![
        "python3".to_string(),
        "-m".to_string(),
        "sglang.launch_server".to_string(),
        "--model-path".to_string(),
        mount_dst,
        "--host".to_string(),
        "0.0.0.0".to_string(),
        "--port".to_string(),
        port.to_string(),
    ];
    push_sglang_args(&mut args, vllm_args, ctx_size);
    push_vllm_flags(&mut args, vllm_flags);
    let n_nodes = mn.nodes.len();
    args.extend([
        "--nnodes".to_string(),
        n_nodes.to_string(),
        "--node-rank".to_string(),
        "0".to_string(),
        "--tp".to_string(),
        n_nodes.to_string(),
        "--dist-init-addr".to_string(),
        format!("{}:{}", mn.nodes[0].ip, mn.dist_init_port),
    ]);
    args
}

/// 多机 worker（远端 rank i）SGLang `launch_server` 命令参数：与 head 同命令，仅 `--node-rank i`
/// （SGLang 无 vLLM 式 `--headless`——各节点跑同一 server，仅 rank 0 对外提供服务）。
fn build_multi_node_worker_sglang_args(
    model_id: &str,
    port: u16,
    vllm_args: &VllmArgs,
    vllm_flags: &Option<Vec<String>>,
    rank: usize,
    mn: &MultiNodeArgs,
    ctx_size: Option<i32>,
) -> Vec<String> {
    let mount_dst = format!("/models/{}", model_id);
    let mut args = vec![
        "python3".to_string(),
        "-m".to_string(),
        "sglang.launch_server".to_string(),
        "--model-path".to_string(),
        mount_dst,
        "--host".to_string(),
        "0.0.0.0".to_string(),
        "--port".to_string(),
        port.to_string(),
    ];
    push_sglang_args(&mut args, vllm_args, ctx_size);
    push_vllm_flags(&mut args, vllm_flags);
    let n_nodes = mn.nodes.len();
    args.extend([
        "--nnodes".to_string(),
        n_nodes.to_string(),
        "--node-rank".to_string(),
        rank.to_string(),
        "--tp".to_string(),
        n_nodes.to_string(),
        "--dist-init-addr".to_string(),
        format!("{}:{}", mn.nodes[0].ip, mn.dist_init_port),
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
    container_name: &str,
    model_dir: &std::path::Path,
    image: &str,
    shm_size: &str,
    _node0_ip: &str,
    mn: &MultiNodeArgs,
    node_ip: &str,
    iface: Option<&str>,
    ib_iface: Option<&str>,
    has_infiniband: bool,
    model_env: &[String],
    extra_mounts: &[(String, String, bool)],
) -> Vec<String> {
    let mount_dst = format!("/models/{}", model_id);
    let mut args = build_common_docker_prefix(container_name, shm_size, node_ip, iface, ib_iface, has_infiniband, &mn.extra_env, model_env);
    args.push("-v".to_string());
    args.push(format!("{}:{}:ro", model_dir.to_string_lossy(), mount_dst));
    // vllm_extra_mounts：附加模型目录（drafter 等），主挂载之后逐条追加，空列表零变化
    for (host_path, container_path, read_only) in extra_mounts {
        args.push("-v".to_string());
        let mode = if *read_only { ":ro" } else { "" };
        args.push(format!("{}:{}{}", host_path, container_path, mode));
    }
    // fork 默认 `--entrypoint=` 清空镜像 ENTRYPOINT（避免 nvidia_entrypoint.sh 触发），
    // 命令仅 `sleep infinity`，让容器保活等待后续 docker exec 启动 Ray worker
    args.push("--entrypoint=".to_string());
    args.push(image.to_string());
    args.push("sleep".to_string());
    args.push("infinity".to_string());
    args
}

/// 多机容器名前缀（vLLM / SGLang 各一套，供停止/监控/清理同源推导）
fn multi_container_base(engine_label: &str, model_id: &str) -> String {
    if engine_label == "sglang" {
        format!("adm-sglang-{}", model_id)
    } else {
        format!("adm-vllm-{}", model_id)
    }
}

/// 多机容器名 `<base>-rank-<R>`（本机 rank 0；停止时据此识别多机模式）
fn multi_container_name(container_base: &str, rank: usize) -> String {
    format!("{}-rank-{}", container_base, rank)
}

/// 停止已启动的远端节点容器（启动失败回滚 / 停止模型共用）
async fn stop_remote_containers(
    app: &tauri::AppHandle,
    mn: &MultiNodeArgs,
    key: Option<&str>,
    container_base: &str,
    ranks_start: usize,
) {
    for (i, node) in mn.nodes.iter().enumerate().skip(ranks_start) {
        if node.is_self {
            continue;
        }
        // 从容器前缀还原 model_id（日志/前端事件按 model_id 关联卡片）
        let model_id = container_base
            .strip_prefix("adm-sglang-")
            .or_else(|| container_base.strip_prefix("adm-vllm-"))
            .unwrap_or(container_base);
        let container = multi_container_name(container_base, i);
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
    extra_mounts: Option<Vec<String>>,
    engine: Option<String>,
) -> Result<(), AppError> {
    let (mn, mut vllm_args) = load_multi_node_config(app);
    // 引擎：缺省 vLLM；sglang 时容器名 / 命令 / 就绪信号 / 预检走 SGLang 分支
    let is_sglang = engine
        .as_deref()
        .map(|s| s.trim().eq_ignore_ascii_case("sglang"))
        .unwrap_or(false);
    let engine_tag = if is_sglang { "sglang" } else { "vllm" };
    let engine_display = if is_sglang { "SGLang" } else { "vLLM" };
    let container_base = multi_container_base(engine_tag, model_id);
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

    // 本地引擎日志落盘：<data_dir>/logs/adm_<engine>_<model_id>_rank_0.log
    // 每行输出实时写文件（每行 flush），容器被清理后日志依然可查——
    // 排查"启动即失败被 docker rm 删掉、原因无处可看"的关键。
    let local_vllm_log = crate::common::config::get_data_dir(Some(app))?
        .join("logs")
        .join(format!("adm_{}_{}_rank_0.log", engine_tag, model_id));
    if let Some(parent) = local_vllm_log.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let log_writer = std::sync::Arc::new(std::sync::Mutex::new(
        std::io::BufWriter::new(
            std::fs::File::create(&local_vllm_log)
                .map_err(|e| AppError::msg(format!("创建引擎日志文件失败（{}）: {}", local_vllm_log.display(), e)))?,
        ),
    ));

    // 镜像由远程 model.json 的 engine_image / vllm_image 字段唯一指定（每模型独立配置），缺字段视为清单错误。
    let image = vllm_image
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::msg(format!("模型 {} 缺少镜像配置（远程 model.json 的 engine_image / vllm_image 必填）", model_id)))?
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
    // vllm_extra_mounts：head（本机）附加模型目录提前解析 + 校验（空清单零开销）。
    // 放在 worker 启动之前：缺失时快速失败，避免远端容器已拉起后才发现本地缺文件。
    let head_extra_mounts = resolve_extra_mounts(
        model_dir.parent().unwrap_or(model_dir),
        extra_mounts.as_deref().unwrap_or(&[]),
        true,
    )?;
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
        // vllm_extra_mounts：附加模型目录（drafter 等）远端探活 + 解析挂载参数（空清单零开销）
        let node_extra_mounts = probe_remote_extra_mounts(node, key_ref, extra_mounts.as_deref().unwrap_or(&[])).await?;
        let _ = app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": format!("[多机] 节点 {}（rank {}）环境正常：GPU={} Docker={} 镜像={}", node.ip, i, gpu, "OK", "OK"),
            "source": "stdout",
        }));

        // 启动：nohup docker run 后台运行，日志落盘 /tmp/adm_<engine>_<model>_rank_<i>.log
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
        let container = multi_container_name(&container_base, i);
        let log_path = format!("/tmp/adm_{}_{}_rank_{}.log", engine_tag, model_id, i);
        let args = build_multi_node_worker_args(
            model_id,
            &container,
            &std::path::Path::new(&node_model_dir),
            &image,
            &shm_size,
            &node0_ip,
            &mn,
            &node.ip,
            remote_iface.as_deref(),
            remote_ib_iface.as_deref(),
            remote_has_ib,
            vllm_env.as_deref().unwrap_or(&[]),
            &node_extra_mounts,
        );
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
            let _ = stop_remote_containers(app, &mn, key_ref, &container_base, 1).await;
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
            let _ = stop_remote_containers(app, &mn, key_ref, &container_base, 1).await;
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
                format!("容器状态：{}（日志为空，docker run 阶段即失败，可在远端查看 /tmp/adm_{}_{}_rank_{}.log）", st, engine_tag, model_id, i)
            } else {
                let hint = if poll_fail > 0 { format!("远端 SSH 轮询失败 {} 次；", poll_fail) } else { String::new() };
                format!("{}30s 内未进入运行状态（未看到容器，docker run 可能立即失败，可在远端查看 /tmp/adm_{}_{}_rank_{}.log）", hint, engine_tag, model_id, i)
            };
            crate::common::utils::logger::write_log("ERROR", "MODEL", &format!("[{}] 远端节点 {} 容器启动失败:\n{}", model_id, node.ip, detail));
            bail!("远端节点 {}（rank {}）容器启动失败，已回滚停止已启动节点：\n{}", node.ip, i, detail);
        }
        let _ = app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": format!("[多机] 远端节点 {}（rank {}）容器已就绪", node.ip, i),
            "source": "stdout",
        }));
        // worker 容器内引擎可用性预检：缺失时 serve 进程秒退、head 阻塞等 rank join（前端只见超时）。
        // vLLM 查 vllm 命令；SGLang 查 sglang 包（find_spec，避免重导入）。失败即回滚。
        {
            let probe_cmd = if is_sglang {
                let inner = "if python3 -c \"import importlib.util,sys; sys.exit(0 if importlib.util.find_spec('sglang') else 1)\" >/dev/null 2>&1; then echo SGLANG_PKG_OK; else echo SGLANG_PKG_MISSING; echo \"PATH=$PATH\"; command -v vllm >/dev/null 2>&1 && echo IMAGE_IS_VLLM; fi";
                format!(
                    "{} exec {} bash -c {}",
                    if use_sudo { "sudo -n docker" } else { "docker" },
                    crate::common::ssh::sh_quote(&container),
                    crate::common::ssh::sh_quote(inner)
                )
            } else {
                format!(
                    "{} exec {} bash -c 'if command -v vllm >/dev/null 2>&1; then echo VLLM_BIN_OK; else echo VLLM_BIN_MISSING; echo \"PATH=$PATH\"; command -v sglang >/dev/null 2>&1 && echo IMAGE_IS_SGLANG; ls /usr/local/bin /usr/bin /opt/venv/bin /opt/conda/bin 2>/dev/null | grep -i vllm | head -5; fi'",
                    if use_sudo { "sudo -n docker" } else { "docker" },
                    crate::common::ssh::sh_quote(&container)
                )
            };
            let (pok, pout, perr) = crate::common::ssh::ssh_run(
                &node.ip, &node.ssh_user, node.ssh_port, key_ref, &probe_cmd,
                std::time::Duration::from_secs(15),
            ).await.unwrap_or((false, String::new(), String::new()));
            let engine_missing = if is_sglang { pout.contains("SGLANG_PKG_MISSING") } else { pout.contains("VLLM_BIN_MISSING") };
            if !pok || engine_missing {
                let _ = stop_remote_containers(app, &mn, key_ref, &container_base, 1).await;
                let diag: Vec<&str> = pout.lines()
                    .filter(|l| l.contains("PATH=") || l.to_lowercase().contains("vllm") || l.to_lowercase().contains("sglang"))
                    .collect();
                let diag_str = if diag.is_empty() { String::new() } else { format!("（{}\n）", diag.join("\n")) };
                let missing_token = if is_sglang { "sglang 包" } else { "vllm 命令" };
                let hint = if is_sglang {
                    if pout.contains("IMAGE_IS_VLLM") {
                        "该镜像内是 vllm 而非 sglang（若这是 vLLM 模型请去掉清单里的 \"engine\": \"sglang\"）"
                    } else {
                        "镜像可能为旧构建或经镜像加速器缓存（浮动 tag 内容不一致）"
                    }
                } else if pout.contains("IMAGE_IS_SGLANG") {
                    "该镜像内是 sglang 而非 vllm（若这是 SGLang 模型请在清单标注 \"engine\": \"sglang\"；否则请改用真正的 vLLM 镜像）"
                } else {
                    "镜像可能为旧构建或经镜像加速器缓存（浮动 tag 内容不一致）"
                };
                let detail = if engine_missing {
                    format!("容器内找不到 {}。{}。{}", missing_token, hint, diag_str)
                } else {
                    format!("容器内 {} 预检执行失败", missing_token)
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
    let container0 = multi_container_name(&container_base, 0);
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
        model_id, &container0, model_dir, &image, &shm_size, &mn, &node0_ip, local_iface.as_deref(), local_ib_iface.as_deref(), local_has_ib,
        vllm_env.as_deref().unwrap_or(&[]),
        &head_extra_mounts,
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
            stop_remote_containers(app, &mn, key_ref, &container_base, 1).await;
            return Err(AppError::msg(format!("head 容器启动失败: {}", stderr.trim())));
        }
    } else {
        let _ = crate::common::utils::platform::docker_cmd()
            .args(["rm", "-f", &container0])
            .output();
        stop_remote_containers(app, &mn, key_ref, &container_base, 1).await;
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
        stop_remote_containers(app, &mn, key_ref, &container_base, 1).await;
        return Err(AppError::msg("head 容器 30s 内未进入 Up 状态".to_string()));
    }
    let _ = app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("[多机] 本机（rank 0）容器已就绪"),
        "source": "stdout",
    }));

    // ===== Phase 2.5: head 容器内引擎可用性预检 =====
    // 镜像缺引擎时 Phase 4 的命令只会留下 `command not found` 然后 head 阻塞等 rank join——
    // 提前探测，失败即回滚并给出可操作的修复指引（重拉镜像）。
    {
        let probe = if is_sglang {
            crate::common::utils::platform::docker_cmd()
                .args(["exec", &container0, "bash", "-c",
                       "if python3 -c \"import importlib.util,sys; sys.exit(0 if importlib.util.find_spec('sglang') else 1)\" >/dev/null 2>&1; then echo SGLANG_PKG_OK; else echo SGLANG_PKG_MISSING; command -v vllm >/dev/null 2>&1 && echo IMAGE_IS_VLLM; fi"])
                .output()
        } else {
            crate::common::utils::platform::docker_cmd()
                .args(["exec", &container0, "bash", "-c",
                       "if command -v vllm >/dev/null 2>&1; then echo VLLM_BIN_OK; else echo VLLM_BIN_MISSING; command -v sglang >/dev/null 2>&1 && echo IMAGE_IS_SGLANG; fi"])
                .output()
        };
        if let Ok(o) = &probe {
            let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
            let engine_missing = if is_sglang { stdout.contains("SGLANG_PKG_MISSING") } else { stdout.contains("VLLM_BIN_MISSING") };
            if engine_missing {
                let missing_token = if is_sglang { "sglang 包" } else { "vllm 命令" };
                let hint = if is_sglang {
                    if stdout.contains("IMAGE_IS_VLLM") {
                        "（该镜像内是 vllm 而非 sglang：若这是 vLLM 模型请去掉清单里的 \"engine\": \"sglang\"）"
                    } else {
                        ""
                    }
                } else if stdout.contains("IMAGE_IS_SGLANG") {
                    "（该镜像内是 sglang 而非 vllm：若这是 SGLang 模型请在清单标注 \"engine\": \"sglang\"；否则此 tag 实际被 SGLang 镜像占用/打错，请改用真正的 vLLM 镜像）"
                } else {
                    ""
                };
                let diag = crate::common::utils::platform::docker_cmd()
                    .args(["exec", &container0, "bash", "-c",
                           "echo PATH=$PATH; ls /usr/local/bin /usr/bin 2>/dev/null | grep -iE 'vllm|sglang' | head -3"])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                let _ = app.emit("model-log", serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[ERROR] 镜像 {} 容器内找不到 {}{}（{}）", image, missing_token, hint, if diag.is_empty() { format!("容器内无 {} 相关文件", missing_token) } else { diag }),
                    "source": "stderr",
                }));
                let _ = crate::common::utils::platform::docker_cmd()
                    .args(["rm", "-f", &container0])
                    .output();
                stop_remote_containers(app, &mn, key_ref, &container_base, 1).await;
                return Err(AppError::msg(format!(
                    "镜像 {} 容器内找不到 {}（镜像内容可能不完整或被镜像加速器替换）：请执行 `docker rmi {}` 后在模型列表重新下载模型触发重新拉取，已回滚停止集群",
                    image, missing_token, image
                )));
            }
        }
    }

    // ===== Phase 3: 派发远端 worker 引擎进程（vLLM：no-Ray mp / SGLang：launch_server 各 rank）=====
    // vLLM：fork 镜像 pydantic 校验 nnodes>1 只允许 mp/uni/external_launcher，worker 与 head 都用
    // `--distributed-executor-backend mp`（build 函数末尾强推，覆盖用户 vllm_flags 里可能自带的 ray）。
    // 两种引擎都先起 worker（docker exec -d 后台，只派发不等待），head 随后前台启动并等所有 rank join。
    let worker_serve_label = if is_sglang { "sglang.launch_server" } else { "vllm serve --headless" };
    let _ = app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("[多机] 派发远端 worker 的 {}（rank 1..{}）", worker_serve_label, n_nodes - 1),
        "source": "stdout",
    }));
    for (i, container, use_sudo, _log_path) in &worker_runtime {
        let node = &mn.nodes[*i];
        let worker_vllm_args = if is_sglang {
            build_multi_node_worker_sglang_args(
                model_id, port, &vllm_args, &vllm_flags, *i, &mn, params.ctx_size,
            )
        } else {
            build_multi_node_worker_vllm_args(
                model_id, port, &vllm_args, &vllm_flags, *i, &mn, params.ctx_size,
            )
        };
        let start_script = crate::common::ssh::start_worker_script(
            container, &worker_vllm_args, *use_sudo,
        );
        match crate::common::ssh::ssh_run(
            &node.ip, &node.ssh_user, node.ssh_port, key_ref, &start_script,
            std::time::Duration::from_secs(15),
        ).await {
            Ok((true, _, _)) => {
                let _ = app.emit("model-log", serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[多机] 远端节点 {}（rank {}）{} 已派发", node.ip, *i, worker_serve_label),
                    "source": "stdout",
                }));
            }
            Ok((false, _, err)) => {
                crate::common::utils::logger::write_log(
                    "WARN", "MODEL",
                    &format!("[{}] 远端节点 {}（rank {}）worker 派发失败: {}", model_id, node.ip, *i, err),
                );
                let _ = app.emit("model-log", serde_json::json!({
                    "model_id": model_id,
                    "line": format!("[多机] 远端节点 {}（rank {}）worker 派发失败（继续，head 端可能超时）", node.ip, *i),
                    "source": "stderr",
                }));
            }
            Err(e) => {
                crate::common::utils::logger::write_log(
                    "WARN", "MODEL",
                    &format!("[{}] 远端节点 {}（rank {}）worker 派发异常: {}", model_id, node.ip, *i, e),
                );
            }
        }
    }

    // ===== Phase 4: 在 head 容器内 exec 引擎命令（本地 docker exec -i 捕获 stdout）=====
    // 用 build_head_*_exec_args 拼命令 tokens；经 sh_quote + bash -c 串成一行，整段塞进
    // `docker exec -i <head_container> bash -c "<cmd>"`，child stdout/stderr 仍归我们管。
    let vllm_exec_args = if is_sglang {
        build_head_sglang_exec_args(model_id, port, &vllm_args, &vllm_flags, &mn, params.ctx_size)
    } else {
        build_head_vllm_exec_args(model_id, port, &vllm_args, &vllm_flags, &mn, params.ctx_size)
    };
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
        "line": format!("[多机] 本机（rank 0）启动 {}（docker exec -i）", if is_sglang { "sglang.launch_server（多机模式）" } else { "vllm serve（mp 多机模式）" }),
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
                let msg = format!("启动引擎进程失败: {}", e);
                let _ = app.emit("model-log", serde_json::json!({
                    "model_id": model_id, "line": format!("[ERROR] {}", msg), "source": "stderr",
                }));
                let _ = crate::common::utils::platform::docker_cmd()
                    .args(["rm", "-f", &container0]).output();
                stop_remote_containers(app, &mn, key_ref, &container_base, 1).await;
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
    {
        let mut engine_lock = state.running_engine.lock().map_err(|e| e.to_string())?;
        *engine_lock = Some(engine_tag.to_string());
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
    let container_base_clone = container_base.clone();

    // 提示用户本地日志路径（便于事后排查）
    let _ = app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("[多机] {} 日志实时落盘：{}", engine_display, local_vllm_log.display()),
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
                    crate::common::utils::logger::write_log("INFO", engine_tag, &line);
                    if let Ok(mut w) = lw.lock() {
                        let _ = writeln!(w, "{}", line);
                        let _ = w.flush();
                    }
                    app_c.emit("model-log", serde_json::json!({
                        "model_id": &mid, "line": line.clone(), "source": "stdout",
                    })).ok();
                    let ready = if is_sglang {
                        line.contains("The server is fired up and ready to roll!")
                    } else {
                        line.contains("Uvicorn running on")
                            || line.contains("Application startup complete")
                            || line.contains("Starting vLLM API server")
                    };
                    if ready {
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
                    crate::common::utils::logger::write_log("WARN", engine_tag, &line);
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
                crate::common::utils::logger::write_log("ERROR", engine_tag, &tail);
                app_clone2.emit("model-log", serde_json::json!({
                    "model_id": &model_id_clone2,
                    "line": format!("[{} exited] 容器日志尾部（清理前抓取）:\n{}", engine_tag, tail),
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
            let c = multi_container_name(&container_base_clone, i);
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
            *state.running_engine.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
        let container_base_m = container_base.clone();
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
                    let c = multi_container_name(&container_base_m, i);
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
                            let log_path = format!("/tmp/adm_{}_{}_rank_{}.log", engine_tag, mid, i);
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
/// ComfyUI 应用服务（「视频生成」页入口，`engine: "comfyui"` 专用）：
/// - 权重由 ComfyUI 自身管理（首次打开官方模板自动下载到挂载目录），**不挂载 `/models/<id>`**；
/// - 挂载清单来自模型清单 `vllm_extra_mounts`，默认**读写**（模型/输出/输入/用户目录），
///   条目可用 `:ro` / `:rw` 显式覆盖；
/// - 容器内命令 = 模型清单 `engine_command`（缺省 `python3 main.py`）+ 自动补 `--listen 0.0.0.0`
///   （未显式给出时）+ `--port <页面端口>` + 模型清单 `vllm_flags`（最后追加、优先级最高）；
/// - 设置页「模型启动参数」（LLM 专属子集）整体跳过；
/// - 就绪：`engine_ready_probe` + `engine_ready_path`（缺省 `/system_stats`；ComfyUI 无 `/health`）；
/// - 容器名 `adm-comfyui-<model_id>`，`--ipc host` + `--shm-size`（DGX 默认 16g，其他 8g）。
async fn start_comfyui_docker(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
    model_id: &str,
    models_dir: &std::path::Path,
    params: LaunchParams,
    device: Option<String>,
    engine_image: Option<String>,
    engine_command: Option<Vec<String>>,
    engine_flags: Option<Vec<String>>,
    engine_env: Option<Vec<String>>,
    extra_mounts: Option<Vec<String>>,
    ready_probe: bool,
    ready_path: Option<String>,
) -> Result<(), AppError> {
    const CONTAINER_PREFIX: &str = "adm-comfyui-";
    let container_name = format!("{}{}", CONTAINER_PREFIX, model_id);

    // 设置页仅取 extra_env / shm_size（LLM 参数子集不适用于 ComfyUI）
    let settings_path = config::get_data_dir(Some(app))?.join("config.json");
    let mut vllm_args = VllmArgs::default();
    if let Ok(json) = std::fs::read_to_string(&settings_path) {
        if let Ok(parsed) = serde_json::from_str::<Settings>(&json) {
            vllm_args = parsed.vllm_args;
        }
    }

    let image = engine_image
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::msg(format!("模型 {} 缺少镜像配置（ComfyUI 引擎需在远程 model.json 指定 engine_image）", model_id)))?
        .to_string();
    crate::common::utils::logger::write_log("INFO", "DOCKER", &format!("[{}] 使用 ComfyUI 镜像 {}", model_id, image));
    app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("使用镜像 {}（引擎 ComfyUI，模型 engine_image）", image),
        "source": "stdout",
    })).ok();
    let default_shm = match device.as_deref() {
        Some("dgx-spark-128G") => "16g",
        _ => "8g",
    };
    let shm_size = if vllm_args.shm_size.is_empty() { default_shm.to_string() } else { vllm_args.shm_size.clone() };

    // ===== 启动前 Docker 环境预检（CLI / daemon / GPU runtime）+ 镜像就绪 =====
    // ComfyUI 镜像由手工脚本构建并推送到 registry（应用内不再提供构建流程），
    // 缺失时当场拉取（对齐其它引擎，进度经 model-pull-progress 驱动页内显示）
    pull_docker_image(app, model_id, &image).await?;

    // 镜像内 ComfyUI 预检（--entrypoint bash 绕开镜像 ENTRYPOINT，无需 GPU）
    {
        let probe = crate::common::utils::platform::docker_cmd_tokio()
            .args(["run", "--rm", "--entrypoint", "/bin/bash", &image, "-c",
                   "if [ -f /opt/ComfyUI/main.py ]; then echo COMFY_OK; else echo COMFY_MISSING; fi"])
            .output();
        if let Ok(Ok(o)) = tokio::time::timeout(std::time::Duration::from_secs(60), probe).await {
            let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
            if stdout.contains("COMFY_MISSING") {
                let msg = format!(
                    "镜像 {} 内未找到 ComfyUI（/opt/ComfyUI/main.py 缺失）：请检查 engine_image 指向的镜像（手工脚本 scripts/docker/h3-comfyui/ 构建并推送到 registry 的产物）",
                    image
                );
                app.emit("model-log", serde_json::json!({
                    "model_id": model_id, "line": format!("[ERROR] {}", msg), "source": "stderr",
                })).ok();
                bail!("{}", msg);
            }
        }
    }

    let port: u16 = params.port.unwrap_or(8188);
    {
        let probe = std::net::TcpListener::bind(("0.0.0.0", port));
        if probe.is_err() {
            bail!("端口 {} 已被占用，请先关闭占用该端口的进程，或在「视频生成」页更换端口", port);
        }
    }

    // 清理同名残留容器
    let _ = crate::common::utils::platform::docker_cmd()
        .args(["rm", "-f", &container_name])
        .output();

    // 附加挂载：ComfyUI 默认读写（权重/输出/输入/用户目录），`:ro` 条目保持只读
    let extra_mounts = resolve_extra_mounts(models_dir, extra_mounts.as_deref().unwrap_or(&[]), false)?;
    if extra_mounts.is_empty() {
        app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": "[警告] 模型清单未配置 vllm_extra_mounts：ComfyUI 权重与产物将落在容器内，容器删除后丢失",
            "source": "stderr",
        })).ok();
    }

    // 设置页「额外环境变量」+ 模型清单 vllm_env 注入
    let extra_env_args = build_container_env_args(
        app,
        model_id,
        &vllm_args.extra_env,
        engine_env.as_deref().unwrap_or(&[]),
    );

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
    ];
    for (host_path, container_path, read_only) in &extra_mounts {
        args.push("-v".to_string());
        let mode = if *read_only { ":ro" } else { "" };
        args.push(format!("{}:{}{}", host_path, container_path, mode));
    }
    args.extend(extra_env_args);
    // 清空镜像自带 ENTRYPOINT：命令完整自持（ComfyUI main.py）
    args.push("--entrypoint=".to_string());
    args.push(image);

    // ===== 容器内启动命令（engine_command 优先，缺省 python3 main.py）=====
    let mut cmd: Vec<String> = engine_command
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if cmd.is_empty() {
        cmd = vec!["python3".to_string(), "main.py".to_string()];
    }
    if !cmd.iter().any(|s| s == "--listen") {
        cmd.push("--listen".to_string());
        cmd.push("0.0.0.0".to_string());
    }
    cmd.push("--port".to_string());
    cmd.push(port.to_string());
    app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("容器启动命令: {}", cmd.join(" ")),
        "source": "stdout",
    })).ok();
    args.extend(cmd.iter().cloned());

    // 模型清单 vllm_flags（ComfyUI 参数）最后追加，优先级最高
    if let Some(flags) = engine_flags.as_deref().filter(|f| !f.is_empty()) {
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
            app.emit("model-log", serde_json::json!({
                "model_id": model_id,
                "line": format!("[模型配置] 已应用模型清单 vllm_flags（ComfyUI 参数，优先级最高）：{}", applied.join(" ")),
                "source": "stdout",
            })).ok();
            crate::common::utils::logger::write_log("INFO", "MODEL", &format!("[{}] 应用模型清单 vllm_flags（ComfyUI）: {}", model_id, applied.join(" ")));
        }
    }

    dbg_log!("comfyui docker args: {:?}", args);
    app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("[DEBUG] full command: docker {:?}", args),
        "source": "stdout",
    })).ok();

    #[cfg(target_os = "windows")]
    let mut child = crate::common::utils::platform::docker_cmd()
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            let msg = format!("启动 ComfyUI 容器失败: {}", e);
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
        let msg = format!("启动 ComfyUI 容器失败: {}", e);
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
    {
        let mut engine_lock = state.running_engine.lock().map_err(|e| e.to_string())?;
        *engine_lock = Some("comfyui".to_string());
    }
    state.set_model_running(true);
    state.bump_model_generation();

    // HTTP 就绪探活（模型清单 engine_ready_probe）：缺省路径 /system_stats
    if ready_probe {
        let path = {
            let p = ready_path.unwrap_or_default();
            let p = p.trim();
            if p.starts_with('/') { p.to_string() } else { "/system_stats".to_string() }
        };
        spawn_ready_probe(app.clone(), model_id.to_string(), port, path);
    }

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
                    crate::common::utils::logger::write_log("INFO", "ComfyUI", &line);
                    app_c
                        .emit("model-log", serde_json::json!({
                            "model_id": &mid, "line": line.clone(), "source": "stdout",
                        }))
                        .ok();
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
                    crate::common::utils::logger::write_log("WARN", "ComfyUI", &line);
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
                    "line": format!("[DEBUG] ComfyUI 容器退出 with status: {}", status),
                    "source": "stdout",
                })).ok();
            }
            Err(e) => {
                app_clone2.emit("model-log", serde_json::json!({
                    "model_id": &model_id_clone2,
                    "line": format!("[ERROR] ComfyUI 容器等待失败: {}", e),
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
            *state.running_engine.lock().unwrap_or_else(|e| e.into_inner()) = None;
            state.set_model_running(false);
        }

        app_clone2
            .emit("model-stopped", serde_json::json!({ "model_id": &model_id_clone2 }))
            .ok();
    });

    Ok(())
}

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
    extra_mounts: Option<Vec<String>>,
    engine_command: Option<Vec<String>>,
    engine_ready_patterns: Option<Vec<String>>,
    engine_ready_probe: bool,
    engine_ready_path: Option<String>,
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

    // 镜像由远程 model.json 的 engine_image / vllm_image 字段唯一指定（每模型独立配置），本地不再保留硬编码兜底；
    // 缺字段视为模型清单配置错误，直接拒绝启动。
    let image = vllm_image
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::msg(format!("模型 {} 缺少镜像配置（远程 model.json 的 engine_image / vllm_image 必填）", model_id)))?
        .to_string();
    crate::common::utils::logger::write_log("INFO", "DOCKER", &format!("[{}] 使用镜像 {}", model_id, image));
    app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("使用镜像 {}（模型 engine_image / vllm_image）", image),
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
                    "（检测到该镜像内是 sglang 而非 vllm：若这是 SGLang 模型，请在远程 model.json 为该模型标注 \"engine\": \"sglang\"；否则说明此 tag 实际被 SGLang 镜像占用/打错，请改用真正的 vLLM 镜像，如本清单其他模型使用的 ghcr.io/spark-arena/dgx-vllm-eugr-nightly-b12x）"
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

    // 就绪关键字：vLLM 内置标志 + 模型清单 engine_ready_patterns 追加的自定义标志
    let mut ready_patterns: Vec<String> = vec![
        "Application startup complete".to_string(),
        "Starting vLLM API server".to_string(),
        "Uvicorn running on".to_string(),
    ];
    ready_patterns.extend(
        engine_ready_patterns
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    );

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
    // vllm_extra_mounts：附加模型目录（drafter 等）挂载参数（空清单零开销，其他模型 docker 参数不变）
    let extra_mounts = resolve_extra_mounts(
        model_dir.parent().unwrap_or(model_dir),
        extra_mounts.as_deref().unwrap_or(&[]),
        true,
    )?;

    // ===== 设置页「额外环境变量」+ 模型清单 vllm_env 注入（vLLM / SGLang 单机路径共用 helper）=====
    let extra_env_args = build_container_env_args(
        app,
        model_id,
        &vllm_args.extra_env,
        vllm_env.as_deref().unwrap_or(&[]),
    );

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
    // 附加挂载在主模型之后逐条追加（均在 image 之前）
    for (host_path, container_path, read_only) in &extra_mounts {
        args.push("-v".to_string());
        let mode = if *read_only { ":ro" } else { "" };
        args.push(format!("{}:{}{}", host_path, container_path, mode));
    }
    // 注入额外环境变量（必须在 image 之前）
    args.extend(extra_env_args);
    // 清空镜像自带 ENTRYPOINT（如 vllm/vllm-openai 系的 ["vllm","serve"]）：
    // vllm_image 只是镜像名，启动命令完整自持（`vllm serve ...` 原样执行），
    // 绝不与镜像 ENTRYPOINT 拼接（否则命令会被 ENTRYPOINT 吞掉：单机变
    // `vllm serve vllm serve ...`，bash 型 ENTRYPOINT 报 `bash: line 1: vllm: command not found`）
    args.push("--entrypoint=".to_string());
    args.push(image);
    // 容器内启动命令：模型清单 engine_command 优先，缺省 `vllm serve <模型路径>`
    let engine_command: Vec<String> = engine_command
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if engine_command.is_empty() {
        args.push("vllm".to_string());
        args.push("serve".to_string());
        args.push(mount_dst);
    } else {
        app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": format!("使用模型清单 engine_command 作为容器启动命令: {}", engine_command.join(" ")),
            "source": "stdout",
        })).ok();
        args.extend(engine_command.iter().cloned());
        args.push(mount_dst);
    }
    args.push("--host".to_string());
    args.push(host);
    args.push("--port".to_string());
    args.push(port.to_string());

    // 设置页 vLLM 详细参数（仅非空/非默认值才追加）——自定义启动入口（engine_command）
    // 为非 LLM 服务（如 SGLang 扩散 `sglang serve`）时该子集不适用，完整参数由 vllm_flags 承载
    if engine_command.is_empty() {
        push_vllm_args(&mut args, &vllm_args, params.ctx_size);
    } else {
        app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": "已跳过设置页 vLLM 参数子集（模型使用 engine_command 自定义启动入口，参数全部由模型清单 vllm_flags 承载）",
            "source": "stdout",
        })).ok();
    }

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
    {
        let mut engine_lock = state.running_engine.lock().map_err(|e| e.to_string())?;
        *engine_lock = Some("vllm".to_string());
    }
    state.set_model_running(true);
    state.bump_model_generation();

    // HTTP 就绪探活（模型清单 engine_ready_probe）
    if engine_ready_probe {
        spawn_ready_probe(app.clone(), model_id.to_string(), port, engine_ready_path.clone().unwrap_or_default());
    }

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
                    // 推理引擎就绪信号（vLLM 内置标志 + 模型清单 engine_ready_patterns）
                    if ready_patterns.iter().any(|p| line.contains(p.as_str())) {
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
            *state.running_engine.lock().unwrap_or_else(|e| e.into_inner()) = None;
            state.set_model_running(false);
        }

        app_clone2
            .emit("model-stopped", serde_json::json!({ "model_id": &model_id_clone2 }))
            .ok();
    });

    Ok(())
}

/// SGLang Docker 启动（单机）：镜像由模型清单 engine_image 指定（回退 vllm_image），
/// 模型目录只读挂载，容器前台运行（生命周期 = docker run 进程），
/// 就绪信号：stdout 出现 "The server is fired up and ready to roll!"（SGLang 官方就绪标志，
/// 其 uvicorn 启动行早于完全就绪，不能沿用 vLLM 的信号）。
async fn start_sglang_docker(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
    model_id: &str,
    model_dir: &std::path::Path,
    params: LaunchParams,
    device: Option<String>,
    engine_image: Option<String>,
    engine_flags: Option<Vec<String>>,
    engine_env: Option<Vec<String>>,
    extra_mounts: Option<Vec<String>>,
    engine_command: Option<Vec<String>>,
    engine_ready_patterns: Option<Vec<String>>,
    engine_ready_probe: bool,
    engine_ready_path: Option<String>,
) -> Result<(), AppError> {
    const CONTAINER_PREFIX: &str = "adm-sglang-";
    let container_name = format!("{}{}", CONTAINER_PREFIX, model_id);

    // 设置页 vLLM 参数面板对 SGLang 仅取安全子集（push_sglang_args），其余由模型清单 vllm_flags 承载
    let settings_path = config::get_data_dir(Some(app))?.join("config.json");
    let mut sglang_args = VllmArgs::default();
    if let Ok(json) = std::fs::read_to_string(&settings_path) {
        if let Ok(parsed) = serde_json::from_str::<Settings>(&json) {
            sglang_args = parsed.vllm_args;
        }
    }
    sglang_args.distributed_executor_backend.clear();

    let image = engine_image
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::msg(format!("模型 {} 缺少镜像配置（SGLang 引擎需在远程 model.json 指定 engine_image）", model_id)))?
        .to_string();
    crate::common::utils::logger::write_log("INFO", "DOCKER", &format!("[{}] 使用 SGLang 镜像 {}", model_id, image));
    app.emit("model-log", serde_json::json!({
        "model_id": model_id,
        "line": format!("使用镜像 {}（引擎 SGLang，模型 engine_image）", image),
        "source": "stdout",
    })).ok();
    let default_shm = match device.as_deref() {
        Some("dgx-spark-128G") => "64g",
        _ => "32g",
    };
    let shm_size = if sglang_args.shm_size.is_empty() { default_shm.to_string() } else { sglang_args.shm_size.clone() };

    // ===== 启动前 Docker 环境预检（CLI / daemon / GPU runtime / 镜像；返回实际可用镜像名）=====
    let image = check_docker_env(app, model_id, &image).await?;

    // 镜像内 sglang 包可用性预检（--entrypoint bash 绕开镜像 ENTRYPOINT，无需 GPU）：
    // 缺失时给出可操作指引；同时检测 vLLM 镜像误标为 SGLang 的场景。
    {
        let probe = crate::common::utils::platform::docker_cmd_tokio()
            .args(["run", "--rm", "--entrypoint", "/bin/bash", &image, "-c",
                   "if python3 -c \"import importlib.util,sys; sys.exit(0 if importlib.util.find_spec('sglang') else 1)\" >/dev/null 2>&1; then echo SGLANG_PKG_OK; else echo SGLANG_PKG_MISSING; command -v vllm >/dev/null 2>&1 && echo IMAGE_IS_VLLM; fi"])
            .output();
        if let Ok(Ok(o)) = tokio::time::timeout(std::time::Duration::from_secs(60), probe).await {
            let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
            if stdout.contains("SGLANG_PKG_MISSING") {
                let hint = if stdout.contains("IMAGE_IS_VLLM") {
                    "（检测到该镜像内是 vllm 而非 sglang：若这是 vLLM 模型，请去掉远程 model.json 中该模型的 \"engine\": \"sglang\" 配置；否则请改用真正的 SGLang 镜像）"
                } else {
                    ""
                };
                let msg = format!(
                    "镜像 {} 内找不到 sglang 包{}（镜像内容可能不完整或被镜像加速器替换）：请执行 `docker rmi {}` 后在模型列表重新下载模型触发重新拉取",
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
    // Docker 容器内必须监听 0.0.0.0 才能经 -p 端口映射对外服务
    let host = "0.0.0.0".to_string();

    // 就绪关键字：SGLang LLM 内置标志 + 模型清单 engine_ready_patterns（自定义启动入口用，
    // 如扩散服务不打印 LLM banner 时先靠 engine_ready_probe 探活）
    let mut ready_patterns: Vec<String> = vec!["The server is fired up and ready to roll!".to_string()];
    ready_patterns.extend(
        engine_ready_patterns
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    );

    {
        let probe = std::net::TcpListener::bind(("0.0.0.0", port));
        if probe.is_err() {
            bail!(
                "端口 {} 已被占用，请先关闭占用该端口的进程，或在设置页更换监听端口",
                port
            );
        }
    }

    let _ = crate::common::utils::platform::docker_cmd()
        .args(["rm", "-f", &container_name])
        .output();

    let mount_src = model_dir.to_string_lossy().to_string();
    let mount_dst = format!("/models/{}", model_id);
    let extra_mounts = resolve_extra_mounts(
        model_dir.parent().unwrap_or(model_dir),
        extra_mounts.as_deref().unwrap_or(&[]),
        true,
    )?;

    let extra_env_args = build_container_env_args(
        app,
        model_id,
        &sglang_args.extra_env,
        engine_env.as_deref().unwrap_or(&[]),
    );

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
    for (host_path, container_path, read_only) in &extra_mounts {
        args.push("-v".to_string());
        let mode = if *read_only { ":ro" } else { "" };
        args.push(format!("{}:{}{}", host_path, container_path, mode));
    }
    args.extend(extra_env_args);
    args.push("--entrypoint=".to_string());
    args.push(image);
    // 容器内启动命令：模型清单 engine_command 优先（如扩散/视频服务 `["sglang","serve"]`），
    // 缺省 = SGLang LLM 服务 `python3 -m sglang.launch_server`。
    // 后续统一追加 `--model-path/--host/--port`，两种入口参数名一致。
    let engine_command: Vec<String> = engine_command
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if engine_command.is_empty() {
        args.push("python3".to_string());
        args.push("-m".to_string());
        args.push("sglang.launch_server".to_string());
    } else {
        app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": format!("使用模型清单 engine_command 作为容器启动命令: {}", engine_command.join(" ")),
            "source": "stdout",
        })).ok();
        args.extend(engine_command.iter().cloned());
    }
    args.push("--model-path".to_string());
    args.push(mount_dst);
    args.push("--host".to_string());
    args.push(host);
    args.push("--port".to_string());
    args.push(port.to_string());

    // 设置页参数子集（ctx_size → --context-length / tp / mem-fraction-static / trust-remote-code）：
    // 仅在默认 LLM 启动入口生效；自定义 engine_command（如扩散服务 `sglang serve`）下这些
    // LLM 专属参数会被拒绝，整体跳过，完整参数由模型清单 vllm_flags 承载
    if engine_command.is_empty() {
        push_sglang_args(&mut args, &sglang_args, params.ctx_size);
    } else {
        app.emit("model-log", serde_json::json!({
            "model_id": model_id,
            "line": "已跳过设置页 SGLang 参数子集（模型使用 engine_command 自定义启动入口）",
            "source": "stdout",
        })).ok();
    }

    // 模型清单 vllm_flags（SGLang 配方，如 --tp / --context-length / --json-model-override-args /
    // --reasoning-parser ling3 等）最后追加，优先级最高
    if let Some(flags) = engine_flags.as_deref().filter(|f| !f.is_empty()) {
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
                    "line": format!("[模型配置] 已应用模型清单 vllm_flags（SGLang 参数，优先级最高）：{}", applied.join(" ")),
                    "source": "stdout",
                }),
            )
            .ok();
            crate::common::utils::logger::write_log("INFO", "MODEL", &format!("[{}] 应用模型清单 vllm_flags（SGLang）: {}", model_id, applied.join(" ")));
        }
    }

    dbg_log!("sglang docker args: {:?}", args);

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
    {
        let mut engine_lock = state.running_engine.lock().map_err(|e| e.to_string())?;
        *engine_lock = Some("sglang".to_string());
    }
    state.set_model_running(true);
    state.bump_model_generation();

    // HTTP 就绪探活（模型清单 engine_ready_probe）：扩散/视频类服务无固定就绪 banner
    if engine_ready_probe {
        spawn_ready_probe(app.clone(), model_id.to_string(), port, engine_ready_path.clone().unwrap_or_default());
    }

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
                    // 就绪信号：SGLang LLM 官方标志（uvicorn 启动行早于完全就绪，不作为就绪条件），
                    // 以及模型清单 engine_ready_patterns 追加的自定义标志
                    if ready_patterns.iter().any(|p| line.contains(p.as_str())) {
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
            *state.running_engine.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
    extra_mounts: Option<Vec<String>>,
    engine: Option<String>,
    engine_image: Option<String>,
    engine_command: Option<Vec<String>>,
    engine_ready_patterns: Option<Vec<String>>,
    engine_ready_probe: Option<bool>,
    engine_ready_path: Option<String>,
) -> Result<(), AppError> {
    // 统一捕获启动失败并写入本地日志
    let result = start_model_inner(&app, &state, &model_id, params, device, vllm_image, vllm_flags, vllm_env, extra_mounts, engine, engine_image, engine_command, engine_ready_patterns, engine_ready_probe, engine_ready_path).await;
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
    extra_mounts: Option<Vec<String>>,
    engine: Option<String>,
    engine_image: Option<String>,
    engine_command: Option<Vec<String>>,
    engine_ready_patterns: Option<Vec<String>>,
    engine_ready_probe: Option<bool>,
    engine_ready_path: Option<String>,
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
    let engine_ready_probe = engine_ready_probe.unwrap_or(false);

    // ===== ComfyUI 应用服务（「视频生成」页）：无本地模型目录，先于目录模型判断分发 =====
    let is_comfyui = engine
        .as_deref()
        .map(|s| s.trim().eq_ignore_ascii_case("comfyui"))
        .unwrap_or(false);
    if is_comfyui {
        // 有效镜像：engine_image 优先，回退旧字段 vllm_image
        let effective_image = engine_image
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| vllm_image.as_deref().map(str::trim).filter(|s| !s.is_empty()))
            .map(|s| s.to_string());
        return start_comfyui_docker(app, state, model_id, &models_dir, params, device, effective_image, engine_command, vllm_flags, vllm_env, extra_mounts, engine_ready_probe, engine_ready_path).await;
    }

    // ===== 新格式（safetensors 目录模型）：引擎分发（vLLM / SGLang）=====
    let is_dir_model = model_dir.join(".done").exists()
        || (model_dir.join("config.json").exists() && model_dir.join("model.safetensors").exists());
    if is_dir_model {
        // 有效镜像：engine_image 优先，回退旧字段 vllm_image
        let effective_image = engine_image
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| vllm_image.as_deref().map(str::trim).filter(|s| !s.is_empty()))
            .map(|s| s.to_string());
        // 引擎类型：缺省 = vLLM
        let is_sglang = engine
            .as_deref()
            .map(|s| s.trim().eq_ignore_ascii_case("sglang"))
            .unwrap_or(false);
        // 多机互联：设置页启用且节点数 >= 2 时走集群启动（vLLM / SGLang 均支持）
        let (mn, _) = load_multi_node_config(app);
        let multi_node = mn.enabled && mn.nodes.len() >= 2;
        if multi_node {
            return start_multi_node(app, state, model_id, &model_dir, params, effective_image, vllm_flags, vllm_env, extra_mounts, engine).await;
        }
        if is_sglang {
            return start_sglang_docker(app, state, model_id, &model_dir, params, device, effective_image, vllm_flags, vllm_env, extra_mounts, engine_command, engine_ready_patterns, engine_ready_probe, engine_ready_path).await;
        }
        return start_vllm_docker(app, state, model_id, &model_dir, params, device, effective_image, vllm_flags, vllm_env, extra_mounts, engine_command, engine_ready_patterns, engine_ready_probe, engine_ready_path).await;
    }

    // 仅支持 Docker 容器化部署（safetensors 目录模型）
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
                // 远端容器名与运行中容器同源（兼容 vLLM / SGLang 两种前缀）
                let base = container_name.strip_suffix("-rank-0").unwrap_or(&container_name);
                stop_remote_containers(&app, &mn, key.as_deref(), base, 1).await;
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
    {
        let mut engine_lock = state.running_engine.lock().map_err(|e| e.to_string())?;
        *engine_lock = None;
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
