// 本地代理配置（设置页「代理」Tab 的 proxy_url）
//
// 三个用途共用同一份配置：
//   ① 模型 / SD 文件下载：reqwest 客户端挂 Proxy，且镜像策略切 Direct（直接用源链接）；
//   ② docker pull 子进程注入代理 env（rootless / podman 等客户端直连场景兜底）；
//   ③ 标准 dockerd 拉镜像需把代理写入 daemon.json `proxies` 并重启服务
//      （settings.rs 的 save_docker_proxy_config，复用镜像加速的提权安装/重启链路）。

use std::time::Duration;

use crate::common::error::AppError;
use crate::common::utils::download::MirrorPolicy;

const DOWNLOAD_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// 从设置读取代理地址（trim 后；空 = 未配置）。读取失败按未配置处理。
pub async fn proxy_url(app: &tauri::AppHandle) -> String {
    match crate::pages::settings::load_settings(app.clone()).await {
        Ok(s) => s.proxy_url.trim().to_string(),
        Err(_) => String::new(),
    }
}

/// 构造 reqwest 代理（模型 / SD 文件下载客户端用）。
/// 未配置返回 None；地址无效时写 WARN 日志并按未配置处理（不阻塞下载）。
pub async fn reqwest_proxy(app: &tauri::AppHandle) -> Option<reqwest::Proxy> {
    let raw = proxy_url(app).await;
    if raw.is_empty() {
        return None;
    }
    match reqwest::Proxy::all(raw.clone()) {
        Ok(p) => {
            crate::common::utils::logger::write_log("INFO", "PROXY", &format!("模型下载走代理: {}", raw));
            Some(p)
        }
        Err(e) => {
            crate::common::utils::logger::write_log("WARN", "PROXY", &format!("代理地址 {} 无效，下载走直连: {}", raw, e));
            None
        }
    }
}

/// 模型 / SD 文件下载所需的 HTTP 客户端 + 源选择策略（构建一次，各下载点复用）。
pub struct DownloadHttp {
    pub client: reqwest::Client,
    /// 开启代理 → Direct（源链接直下）；否则 HfMirrorFirst（先 hf-mirror 加速）
    pub mirror_policy: MirrorPolicy,
}

/// 构建下载客户端：开启代理时客户端挂代理且源策略为 Direct（避免镜像替换）；
/// `timeout` 传 None 表示不设请求超时（大文件下载可能远超 10 分钟）。
pub async fn build_download_http(
    app: &tauri::AppHandle,
    timeout: Option<Duration>,
) -> Result<DownloadHttp, AppError> {
    let proxy = reqwest_proxy(app).await;
    let mut builder = reqwest::Client::builder().user_agent(DOWNLOAD_UA);
    if let Some(t) = timeout {
        builder = builder.timeout(t);
    }
    if let Some(p) = &proxy {
        builder = builder.proxy(p.clone());
    }
    let client = builder
        .build()
        .map_err(|e| AppError::msg(format!("创建下载客户端失败: {}", e)))?;
    Ok(DownloadHttp {
        client,
        mirror_policy: if proxy.is_some() {
            MirrorPolicy::Direct
        } else {
            MirrorPolicy::HfMirrorFirst
        },
    })
}