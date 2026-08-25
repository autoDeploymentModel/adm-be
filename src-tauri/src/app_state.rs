use std::collections::HashMap;
use std::sync::Mutex;
use sysinfo::System;

pub struct AppState {
    pub running_process: Mutex<Option<u32>>,
    pub running_model_id: Mutex<Option<String>>,
    pub running_port: Mutex<Option<u16>>,
    /// 当前运行的 vLLM Docker 容器名（仅 docker 部署方式使用）
    pub running_container: Mutex<Option<String>>,
    pub downloading_progress: Mutex<HashMap<String, u8>>,
    pub downloading_phase: Mutex<HashMap<String, String>>,
    pub sys: Mutex<System>,
    /// config.json 读-改-写 互斥锁（防止前端操作并发写 config.json 互相覆盖）
    pub config_write_lock: std::sync::Mutex<()>,
    /// 全局标识：是否有模型成功启动
    pub model_running: Mutex<bool>,
    pub model_generation: Mutex<u64>,
    /// 性能测试是否正在进行
    pub benchmark_running: Mutex<bool>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            running_process: Mutex::new(None),
            running_model_id: Mutex::new(None),
            running_port: Mutex::new(None),
            running_container: Mutex::new(None),
            downloading_progress: Mutex::new(HashMap::new()),
            downloading_phase: Mutex::new(HashMap::new()),
            sys: Mutex::new(System::new_all()),
            config_write_lock: std::sync::Mutex::new(()),
            model_running: Mutex::new(false),
            model_generation: Mutex::new(0),
            benchmark_running: Mutex::new(false),
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
