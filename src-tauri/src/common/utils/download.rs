use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use futures_util::StreamExt;
use crate::common::error::AppError;

/// 带断点续传的通用文件下载函数。
///
/// - 如果 `final_path` 已存在，跳过下载（文件已完成）。
/// - 如果 `part_path` 存在，从当前大小处续传（使用 HTTP Range 头）。
/// - 下载完成后，`part_path` 会重命名为 `final_path`。
///
/// 下载源选择策略
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorPolicy {
    /// 直连场景：先试 hf-mirror.com 加速，失败回退 huggingface.co 原链接
    HfMirrorFirst,
    /// 开启代理时直接用源链接（代理已能访问源站，镜像内容还可能滞后，替换无意义）
    Direct,
}

/// 下载被用户取消时返回的错误（用于上层识别并做清理/提示）。
pub fn cancelled_error() -> AppError {
    AppError::Cancelled("下载已取消".to_string())
}

/// 判断错误是否为「用户主动取消下载」。
pub fn is_cancelled(err: &AppError) -> bool {
    matches!(err, AppError::Cancelled(_))
}

/// 带断点续传的通用文件下载函数。
///
/// - 如果 `final_path` 已存在，跳过下载（文件已完成）。
/// - 如果 `part_path` 存在，从当前大小处续传（使用 HTTP Range 头）。
/// - 下载完成后，`part_path` 会重命名为 `final_path`。
///
/// 镜像策略：`MirrorPolicy::HfMirrorFirst` 时传入 `huggingface.co` 原 URL，内部先尝试
/// `hf-mirror.com`（国内加速），连接失败（DNS/连接/TLS）时回退到原始 `huggingface.co`；
/// `MirrorPolicy::Direct`（设置页「代理」开启）时直接用源链接，不做镜像替换。
/// 回退时若有未完成的 `.part` 会先删除（不同源字节布局不同，避免续传错位）。
/// `on_progress` 回调在下载过程中被调用，参数为 `(progress, downloaded, total_size)`：
/// - `progress`: 0-99 的百分比
/// - `downloaded`: 已下载字节数（含续传已有部分）
/// - `total_size`: 文件总大小（未知时为 0）
///
/// `cancel`: 传入取消标志后，每个数据块写入前都会检查；置为 true 时立即停止
/// 下载并返回 `cancelled_error()`（保留 `.part` 文件供下次续传）。
pub async fn download_with_resume(
    client: &reqwest::Client,
    url: &str,
    final_path: &Path,
    part_path: &Path,
    policy: MirrorPolicy,
    cancel: Option<Arc<AtomicBool>>,
    on_progress: impl Fn(u8, u64, u64),
) -> Result<(), AppError> {
    // 镜像优先（国内加速），回退到原 URL；Direct（开启代理）时只用源链接。
    // 不同时持有 .part 续传文件，避免源切换导致字节错位
    let candidates: Vec<String> = match policy {
        MirrorPolicy::HfMirrorFirst => {
            if let Some(rest) = url.strip_prefix("https://huggingface.co/") {
                vec![
                    format!("https://hf-mirror.com/{}", rest),
                    url.to_string(),
                ]
            } else {
                vec![url.to_string()]
            }
        }
        MirrorPolicy::Direct => vec![url.to_string()],
    };

    let mut last_err: Option<AppError> = None;
    for (idx, candidate) in candidates.iter().enumerate() {
        // 切换源时丢弃已有 .part（不同源的同 offset 字节不同，续传会错位）
        if idx > 0 && part_path.exists() {
            let _ = std::fs::remove_file(part_path);
        }
        match try_download_one(client, candidate, final_path, part_path, cancel.clone(), &on_progress).await {
            Ok(()) => return Ok(()),
            Err(e) if is_cancelled(&e) => return Err(e),
            Err(e) if is_network_error(&e) && idx + 1 < candidates.len() => {
                crate::common::utils::logger::write_log(
                    "WARN",
                    "DOWNLOAD",
                    &format!("[{}] 失败，自动切换下一源: {}", candidate, e),
                );
                last_err = Some(e);
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or_else(|| AppError::msg("所有镜像源均失败".to_string())))
}

/// 单一 URL 的下载尝试（含断点续传）
async fn try_download_one(
    client: &reqwest::Client,
    url: &str,
    final_path: &Path,
    part_path: &Path,
    cancel: Option<Arc<AtomicBool>>,
    on_progress: &impl Fn(u8, u64, u64),
) -> Result<(), AppError> {
    // 文件已存在，跳过下载
    if final_path.exists() {
        let _ = std::fs::remove_file(part_path);
        return Ok(());
    }

    // 检查是否有部分下载文件可续传
    let existing_size = if part_path.exists() {
        std::fs::metadata(part_path).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    // 构建请求（带 Range 头续传）
    let mut req = client
        .get(url)
        .header("Accept", "*/*")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8");

    if existing_size > 0 {
        req = req.header("Range", format!("bytes={}-", existing_size));
    }

    let response = req
        .send()
        .await
        .map_err(|e| AppError::msg(format!("下载请求失败: {}", e)))?;

    let status = response.status();
    let is_partial = status == reqwest::StatusCode::PARTIAL_CONTENT;

    // 续传失败（服务器不支持 Range 或文件已变）
    if existing_size > 0 && !is_partial {
        let _ = std::fs::remove_file(part_path);
        return Err(AppError::msg(format!("续传失败 (HTTP {}), 请重新下载", status.as_u16())));
    }

    if !status.is_success() && !is_partial {
        return Err(AppError::msg(format!("下载失败，HTTP 状态码: {}", status.as_u16())));
    }

    // 确定文件总大小
    let total_size = if existing_size > 0 {
        // 从 Content-Range 头解析总大小: "bytes 0-1233/1234"
        response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split('/').nth(1))
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0)
    } else {
        response.content_length().unwrap_or(0)
    };

    // 打开文件（续传追加 / 新建）
    let mut file = if existing_size > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(part_path)
            .await
            .map_err(|e| AppError::msg(format!("打开续传文件失败: {}", e)))?
    } else {
        tokio::fs::File::create(part_path)
            .await
            .map_err(|e| AppError::msg(format!("创建文件失败: {}", e)))?
    };

    // 流式下载
    let mut downloaded: u64 = existing_size;
    let mut stream = response.bytes_stream();

    // 取消轮询：网络停滞（长时间无数据块到达，reqwest 默认无读超时）时，
    // 也能在 500ms 内感知取消标志并立即停止，避免点击后无响应。
    let cancel_poll = async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if cancel.as_ref().map(|f| f.load(Ordering::Relaxed)).unwrap_or(false) {
                break;
            }
        }
    };
    tokio::pin!(cancel_poll);
    let has_cancel = cancel.is_some();

    loop {
        let next = if has_cancel {
            tokio::select! {
                n = stream.next() => n,
                _ = &mut cancel_poll => {
                    return Err(cancelled_error());
                }
            }
        } else {
            stream.next().await
        };
        let Some(chunk_result) = next else { break };
        let chunk = chunk_result.map_err(|e| AppError::msg(format!("下载数据读取失败: {}", e)))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| AppError::msg(format!("写入文件失败: {}", e)))?;
        downloaded += chunk.len() as u64;

        let progress = if total_size > 0 {
            ((downloaded as f64 / total_size as f64) * 100.0).min(99.0) as u8
        } else {
            0
        };
        on_progress(progress, downloaded, total_size);
    }

    file.flush()
        .await
        .map_err(|e| AppError::msg(format!("刷新文件失败: {}", e)))?;
    drop(file);

    // 重命名 .part → 最终文件
    tokio::fs::rename(part_path, final_path)
        .await
        .map_err(|e| AppError::msg(format!("重命名文件失败: {}", e)))?;

    Ok(())
}

/// 判断错误是否属于"网络层失败"（DNS/连接/TLS 等），用于决定是否切换镜像源。
/// HTTP 4xx/5xx 不算网络错误（重试只会拿到同样的状态码）。
fn is_network_error(err: &AppError) -> bool {
    let msg = err.to_string();
    // reqwest 的 connection error / timeout 等都通过 Display 转 string 后含这些关键字
    msg.contains("error sending request")
        || msg.contains("dns error")
        || msg.contains("connection")
        || msg.contains("timeout")
        || msg.contains("tls")
        || msg.contains("SSL")
}
