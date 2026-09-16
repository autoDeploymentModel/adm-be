use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use sysinfo::System;

pub struct AppState {
    pub running_process: Mutex<Option<u32>>,
    pub running_model_id: Mutex<Option<String>>,
    pub running_port: Mutex<Option<u16>>,
    /// 当前运行的 vLLM Docker 容器名（仅 docker 部署方式使用）
    pub running_container: Mutex<Option<String>>,
    /// 当前运行的推理引擎（"vllm" / "sglang"；None = 未运行）
    pub running_engine: Mutex<Option<String>>,
    pub downloading_progress: Mutex<HashMap<String, u8>>,
    pub downloading_phase: Mutex<HashMap<String, String>>,
    /// 正在拉取镜像的 model_id 集合：镜像与权重下载互斥（同一模型不允许并发），
    /// 也用于挡住「重复点击拉镜」
    pub pulling_images: Mutex<HashSet<String>>,
    /// 下载取消标志：model_id → AtomicBool（置 true 后当前下载立即停止，保留 .part 续传）
    pub download_cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub sys: Mutex<System>,
    /// config.json 读-改-写 互斥锁（防止前端操作并发写 config.json 互相覆盖）
    pub config_write_lock: std::sync::Mutex<()>,
    /// 全局标识：是否有模型成功启动
    pub model_running: Mutex<bool>,
    pub model_generation: Mutex<u64>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            running_process: Mutex::new(None),
            running_model_id: Mutex::new(None),
            running_port: Mutex::new(None),
            running_container: Mutex::new(None),
            running_engine: Mutex::new(None),
            downloading_progress: Mutex::new(HashMap::new()),
            downloading_phase: Mutex::new(HashMap::new()),
            pulling_images: Mutex::new(HashSet::new()),
            download_cancel: Mutex::new(HashMap::new()),
            sys: Mutex::new(System::new_all()),
            config_write_lock: std::sync::Mutex::new(()),
            model_running: Mutex::new(false),
            model_generation: Mutex::new(0),
        }
    }

    #[allow(dead_code)]
    pub fn get_running_pid(&self) -> Option<u32> {
        self.running_process.lock().map(|g| *g).unwrap_or(None)
    }

    #[allow(dead_code)]
    pub fn set_running_pid(&self, pid: u32) {
        *self.running_process.lock().unwrap_or_else(|e| e.into_inner()) = Some(pid);
    }

    #[allow(dead_code)]
    pub fn clear_running(&self) {
        *self.running_process.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.running_model_id.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.running_port.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.running_container.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.running_engine.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.model_running.lock().unwrap_or_else(|e| e.into_inner()) = false;
    }

    pub fn set_model_running(&self, running: bool) {
        *self.model_running.lock().unwrap_or_else(|e| e.into_inner()) = running;
    }

    pub fn bump_model_generation(&self) -> u64 {
        let mut g = self.model_generation.lock().unwrap_or_else(|e| e.into_inner());
        *g += 1;
        *g
    }

    #[allow(dead_code)]
    pub fn get_model_generation(&self) -> u64 {
        *self.model_generation.lock().unwrap_or_else(|e| e.into_inner())
    }
}
