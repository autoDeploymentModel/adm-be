// ===== 轻量 i18n：原文即 key =====
// 无框架 / 无打包工具，直接以中文原文作为词典 key。
// 未命中 key 时原样返回中文，保证零遗漏风险（漏译只影响英文用户）。
// 语言持久化：localStorage["adm_lang"]，与 Rust Settings.language 同步。

const EN = {
  // ===== 壳层 index.html =====
  "首页": "Home",
  "设置": "Settings",
  "内存": "Memory",
  "显存": "VRAM",
  "无显卡": "No GPU",
  "加载失败": "Load failed",
  "页面加载中...": "Loading page...",
  "页面加载失败: ": "Failed to load page: ",
  "正在检查更新...": "Checking for updates...",
  "发现新版本": "New version available",
  "有新版本可用，是否前往下载？": "A new version is available. Download it now?",
  "当前版本": "Current version",
  "最新版本": "Latest version",
  "下载更新": "Download update",
  "稍后再说": "Later",
  "已是最新版本": "You are up to date",
  "当前版本 v": "You are on the latest version v",
  "已是最新": "latest",
  "检查更新失败": "Update check failed",
  "下载安装": "Download & Install",
  "请完成安装": "Complete the installation",
  "安装完成": "Done",
  "无法获取下载地址": "Unable to get download URL",
  "关闭": "Close",
  "开始下载": "Start download",
  "准备下载...": "Preparing download...",
  "下载失败: 缺少下载地址": "Download failed: missing download URL",
  "重试": "Retry",
  "下载失败: ": "Download failed: ",
  "正在下载... ": "Downloading... ",
  "正在解压安装...": "Extracting and installing...",
  "安装完成！": "Installation complete!",
  "该功能即将开放，敬请期待": "This feature is coming soon",
  "当前仅支持 Apple Silicon (M 系列) Mac": "Currently only supported on Apple Silicon (M-series) Macs",
  "未安装": "Not installed",

  // ===== settings.js =====
  "返回": "Back",
  "模型启动参数": "Model Launch Params",
  "外观主题": "Appearance",
  "系统版本号": "System Version",
  "关于": "About",
  "推荐模式": "Recommended Mode",
  "选择模式": "Mode",
  "快速配置": "Quick config",
  "默认（日常聊天）": "Default (chat)",
  "创意写作": "Creative writing",
  "写代码 / 编程（推荐）": "Coding (recommended)",
  "选择后自动填充并保存采样参数，可手动微调后自动保存": "Selecting auto-fills and saves sampling params; tweaks are saved automatically",
  "基础参数": "Basic Params",
  "上下文大小": "Context size",
  "预测 token 数": "Tokens to predict",
  "-1 表示无限": "-1 means unlimited",
  "批处理大小": "Batch size",
  "微批次大小": "Micro batch size",
  "GPU 参数": "GPU Params",
  "GPU 层数": "GPU layers",
  "auto (自动)": "auto (automatic)",
  "all (全部)": "all",
  "0 (仅 CPU)": "0 (CPU only)",
  "自定义": "Custom",
  "自定义 GPU 层数": "Custom GPU layers",
  "性能参数": "Performance Params",
  "线程数": "Threads",
  "自动": "auto",
  "留空为自动": "Leave empty for auto",
  "批处理线程数": "Batch threads",
  "同线程数": "Same as threads",
  "KV 缓存类型 K": "KV cache type K",
  "KV 缓存类型 V": "KV cache type V",
  "内存锁定": "Memory lock",
  "强制模型驻留 RAM": "Lock model in RAM",
  "内存映射": "Memory map",
  "启用内存映射": "Enable mmap",
  "采样参数": "Sampling Params",
  "温度": "Temperature",
  "重复惩罚": "Repeat penalty",
  "重复窗口": "Repeat window",
  "重复惩罚的上下文窗口大小，-1 表示使用 ctx_size": "Context window for repeat penalty; -1 uses ctx_size",
  "DRY 乘数": "DRY multiplier",
  "DRY 采样乘数，0.0 表示禁用": "DRY sampling multiplier; 0.0 disables it",
  "DRY 允许长度": "DRY allowed length",
  "DRY 采样允许的重复长度，代码模式建议设为 1": "Allowed repeated length for DRY; 1 is recommended for coding",
  "DRY 惩罚窗口": "DRY penalty window",
  "DRY 惩罚的最后 n 个 token，-1 表示使用上下文大小": "Last n tokens for DRY penalty; -1 uses context size",
  "存在惩罚": "Presence penalty",
  "重复 alpha 存在惩罚，0.0 表示禁用": "Repeat alpha presence penalty; 0.0 disables it",
  "频率惩罚": "Frequency penalty",
  "重复 alpha 频率惩罚，0.0 表示禁用": "Repeat alpha frequency penalty; 0.0 disables it",
  "推理/思考": "Reasoning",
  "auto (自动检测)": "auto (detect)",
  "on (开启)": "on",
  "off (关闭)": "off",
  "控制模型是否启用推理/思考模式": "Controls whether the model uses reasoning mode",
  "服务参数": "Server Params",
  "监听端口": "Listen port",
  "监听地址": "Listen address",
  "127.0.0.1 (本地)": "127.0.0.1 (local)",
  "0.0.0.0 (所有接口)": "0.0.0.0 (all interfaces)",
  "恢复默认": "Reset to default",
  "主题配色": "Theme",
  "选择界面配色，风格参考主流 VS Code 主题，点击即可一键切换（立即生效）": "Pick a color scheme inspired by popular VS Code themes; click to switch instantly",
  "ADM-BE 版本": "ADM-BE version",
  "检测中...": "Checking...",
  "✓ 最新": "✓ Latest",
  "检查新版本": "Check for updates",
  "Tauri 版本": "Tauri version",
  "llama.cpp 版本": "llama.cpp version",
  "删除": "Delete",
  "操作系统": "OS",
  "ADM-BE 是一个大模型部署图形化管理工具，让用户能够便捷地在本地部署和运行大语言模型。": "ADM-BE is a GUI management tool for local LLM deployment, making it easy to deploy and run large language models on your own machine.",
  "如需定制服务 联系方式：微信: litai686": "Custom services: WeChat litai686",
  "项目官网：": "Website: ",
  "删除提示": "Delete Confirmation",
  "当前模式上下文大小不能低于 ": "Context size for this mode cannot be below ",
  "当前模式最小 ": "Minimum for this mode: ",
  "设置已保存，重启模型后生效": "Settings saved; restart the model to apply",
  "保存失败: ": "Save failed: ",
  "未知": "Unknown",
  "未安装或无法检测": "Not installed or undetectable",
  "确定要删除模型文件夹吗？删除后需要重新下载才能使用相关功能。": "Delete the model folder? You will need to re-download it to use related features.",
  "删除失败: ": "Delete failed: ",
  "检查中...": "Checking...",
  "发现新版本 v": "New version v",
  "检查失败: ": "Check failed: ",
  "已切换主题": "Theme applied",

  // ===== settings.js - 网络代理 =====

  // ===== model_list.js =====
  "模型列表": "Model List",
  "模型类型": "Model Type",
  "全部模型": "All Models",
  "正在加载模型列表...": "Loading model list...",
  "确认删除": "Confirm Delete",
  "确定要删除此模型吗？删除后无法恢复。": "Delete this model? This cannot be undone.",
  "确认删除按钮": "Delete",
  "暂无可用模型": "No models available",
  "已启动": "Running",
  "可用": "Available",
  "不可用": "Unavailable",
  "下载 mmproj...": "Downloading mmproj...",
  "下载 diffusion...": "Downloading diffusion...",
  "下载 vae...": "Downloading vae...",
  "继续下载": "Resume",
  "下载": "Download",
  "查看模型": "View Model",
  "关闭模型": "Stop Model",
  "生成图片": "Generate Image",
  "推理": "Reasoning",
  "图片识别": "Vision",
  " · 需内存 ": " · RAM: ",
  " GB": " GB",
  "继续下载中...": "Resuming...",
  "启动中...": "Starting...",
  "启动失败: ": "Start failed: ",
  "停止失败: ": "Stop failed: ",
  "停止中...": "Stopping...",
  "确定要删除模型 \"": "Delete model \"",
  "\" 吗？删除后无法恢复。": "\"? This cannot be undone.",
  "下载失败 [": "Download failed [",
  "]: ": "]: ",
  "模型错误 [": "Model error [",
  "获取模型列表失败: ": "Failed to fetch model list: ",

  // ===== model_image.js =====
  "文生图": "Text-to-Image",
  "正在准备 SD 推理框架...": "Preparing SD inference framework...",
  "检测到 SD 推理框架未安装，正在下载...": "SD inference framework not detected, downloading...",
  "准备中...": "Preparing...",
  "提示词": "Prompt",
  "请输入图片描述...": "Describe the image you want...",
  "宽度": "Width",
  "高度": "Height",
  "生成结果": "Result",
  "💾 另存为": "💾 Save As",
  "生成的图片将显示在这里": "Generated image will appear here",
  "运行日志": "Logs",
  "清空": "Clear",
  "检测到已下载部分文件，继续下载...": "Partial files detected, resuming download...",
  "正在下载 SD 推理框架...": "Downloading SD inference framework...",
  "下载中 ": "Downloading ",
  "正在解压...": "Extracting...",
  "完成": "Done",
  "✓ SD 就绪": "✓ SD ready",
  "⏳ 下载中...": "⏳ Downloading...",
  "✗ 错误": "✗ Error",
  "SD 下载失败: ": "SD download failed: ",
  "SD 初始化失败: ": "SD init failed: ",
  "请输入提示词": "Enter a prompt first",
  "宽度和高度需在 64-4096 之间": "Width and height must be between 64 and 4096",
  "生成中...": "Generating...",
  "正在生成...": "Generating...",
  "获取模型信息失败: ": "Failed to get model info: ",
  "未找到模型文件信息": "Model file info not found",
  "生成失败: ": "Generation failed: ",
  "没有可保存的图片": "No image to save",
  "SD 进程已启动": "SD process started",
  "生成进程已结束": "Generation process ended",
  "生成出错: ": "Generation error: ",
  "未知错误": "Unknown error",
  "文生图 - ": "Text-to-Image - ",

  "需内存 ": "RAM needed: ",
  "语言 / Language": "Language",
  "界面语言": "UI Language",
  "切换后立即生效": "Applies immediately after switching",
  "工具调用": "Tool use",
  "取消": "Cancel",
  "确定": "OK",
  "启动": "Start",
  "适配机型": "Device",
  "全部机型": "All devices",
  "拉取镜像 ": "Pulling image ",
  "切换镜像源重试中...": "Retrying with next mirror...",
  "启动中": "Starting",
  "推理引擎": "Inference Engine",
  "模型最大上下文长度，留 0 表示使用模型自带默认值": "Max context length; 0 uses the model default",
  "Docker 端口映射（容器内始终监听 0.0.0.0）": "Docker port mapping (container always listens on 0.0.0.0)",
  "Docker 部署": "Docker Deployment",
  "镜像": "Image",
  "固定版本 tag，DGX Spark 推荐 lmsysorg/sglang:v0.5.17（多架构自动含 arm64）": "Pin a version tag; lmsysorg/sglang:v0.5.17 recommended for DGX Spark (multi-arch, includes arm64)",
  "共享内存": "Shared Memory",
  "DGX Spark 建议 64g，其他机型 32g": "64g recommended for DGX Spark, 32g otherwise",
  "推理参数": "Inference",
  "张量并行": "Tensor Parallelism",
  "静态内存占比": "Static Memory Fraction",
  "0 = 自动；KV 缓存池/权重内存占比，OOM 时调小": "0 = auto; KV cache/weights memory fraction, lower when OOM",
  "单机多卡时拆分模型权重到多张 GPU（如 2 张卡填 2）；单卡填 1。2 台 Spark 跨机部署需要多节点模式，当前版本暂不支持": "Split model weights across multiple GPUs on one machine (e.g. set 2 for 2 GPUs); set 1 for single GPU. Multi-node across 2 Spark devices requires distributed mode, not supported in this version.",
  "数据类型": "Data Type",
  "auto（默认）": "auto (default)",
  "量化方法": "Quantization",
  "不指定（NVFP4/FP8 模型自动从 config 解析）": "Not set (NVFP4/FP8 models auto-detected from config)",
  "预量化模型（如 unsloth NVFP4）无需指定，加载时自动识别": "Pre-quantized models (e.g. unsloth NVFP4) need no flag; auto-detected",
  "KV Cache 与调度": "KV Cache & Scheduling",
  "KV Cache 类型": "KV Cache Dtype",
  "调度策略": "Schedule Policy",
  "fcfs（默认）": "fcfs (default)",
  "最大运行请求数": "Max Running Requests",
  "0 = 自动": "0 = auto",
  "最大排队请求数": "Max Queued Requests",
  "Chunked Prefill": "Chunked Prefill",
  "0 = 自动，-1 = 禁用；长提示词 OOM 时调小（如 4096）": "0 = auto, -1 = disabled; lower (e.g. 4096) on long-prompt OOM",
  "模型解析与日志": "Parsers & Logging",
  "推理解析器": "Reasoning Parser",
  "无": "None",
  "推理模型（DeepSeek/Qwen3 等）专用，分离思考内容": "For reasoning models (DeepSeek/Qwen3 etc.) to separate thinking content",
  "工具调用解析器": "Tool Call Parser",
  "日志级别": "Log Level",
  "info（默认）": "info (default)",
  "请求日志": "Request Logging",
  "记录所有请求的元数据/输入/输出": "Log metadata/inputs/outputs of all requests",
  "监控指标": "Metrics",
  "启动 Prometheus metrics": "Enable Prometheus metrics",
  "高级自定义参数": "Advanced Custom Args",
  "额外参数": "Extra Args",
  "每行一个 key=value，启动时拼成 --key value 追加到命令尾部；布尔值写 true/false；# 开头为注释": "One key=value per line, appended as --key value; booleans as true/false; lines starting with # are comments",
  "完整参数说明见推理引擎官方文档：docs.sglang.io/docs/advanced_features/server_arguments": "Full argument reference (serving engine docs): docs.sglang.io/docs/advanced_features/server_arguments",

  // ===== settings.js - 推理引擎管理 =====
  "推理引擎管理": "Inference Engine Management",
  "下拉列出本地已拉取的推理引擎版本，选中即保存生效（重启模型后应用）；列表外的版本可在模型清单 sglang-version 指定": "Dropdown lists locally pulled engine versions; selecting applies immediately (effective after model restart). Versions not listed can be pinned per model via the manifest sglang-version",
  "本地已拉取的镜像列表；正在被运行中的模型使用的镜像不可删除": "Locally pulled images; images used by a running model cannot be deleted",
  "版本": "Version",
  "大小": "Size",
  "状态": "Status",
  "操作": "Actions",
  "使用中": "In use",
  "已配置": "Configured",
  "未使用": "Not in use",
  "本地没有已拉取的推理引擎镜像": "No local inference engine images",
  "确认删除镜像 ": "Delete image ",
  "？此操作不可恢复": "? This action cannot be undone",
  "镜像已删除": "Image deleted",
  "列表加载失败: ": "Failed to load list: ",

  // ===== settings.js - 运行日志 =====
  "选择日期": "Date",
  "刷新": "Refresh",
  "打开日志目录": "Open Log Folder",
  "暂无日志": "No logs",
  "加载中...": "Loading...",
  "加载失败: ": "Failed to load: ",
  "打开失败: ": "Failed to open: ",

  // ===== model_list.js - Docker 权限修复 =====
  "Docker 权限不足": "Docker Permission Required",
  "当前用户不在 docker 组，无法启动模型。点击下方按钮，系统会弹出密码框自动修复权限，修复后需重启 ADM-BE 生效。": "The current user is not in the docker group. Click the button below to fix it via a system password prompt. Restart ADM-BE after fixing.",
  "一键修复": "Auto Fix",
  "修复中...": "Fixing...",
  "已修复，请重启": "Fixed, please restart",
  "修复失败: ": "Fix failed: ",
  "已取消，可重新点击修复": "Cancelled, click fix to retry",
  "系统不支持自动修复，请在终端手动执行以下命令，然后重新登录后重启 ADM-BE：": "Auto-fix is not supported on this system. Please run the following command in a terminal, then re-login and restart ADM-BE:",
  "权限修复成功，系统即将自动注销登录，重新登录后即可使用。": "Permission fixed. The system will log out automatically. Please log back in to continue.",
  "清空日志": "Clear Logs",
  "确定清空所有日志吗？": "Are you sure you want to clear all logs?",
  "日志已清空": "Logs cleared",
  "清空失败: ": "Failed to clear: ",

  // ===== benchmark.js =====
  "性能测试": "Benchmark",
  "输入长度 (tokens)": "Input Length (tokens)",
  "输出长度 (tokens)": "Output Length (tokens)",
  "请求数": "Requests",
  "开始测试": "Start Benchmark",
  "测试中...": "Running...",
  "测试启动中...": "Starting benchmark...",
  "点击开始测试，测试结果将显示在此处": "Click start to run benchmark. Results will appear here.",
  "测试启动失败: ": "Failed to start benchmark: ",
  "测试未成功完成": "Benchmark did not complete successfully",
};

let lang = "zh";

/** @param {string} l */
function normalize(l) {
  if (l === "en" || l === "zh") return l;
  return "zh";
}

/** @param {string} l @param {boolean} [persist] 是否写入 localStorage（默认 true；系统语言检测路径传 false，避免固化检测结果） */
export function setLanguage(l, persist) {
  lang = normalize(l);
  if (persist !== false) {
    try { localStorage.setItem("adm_lang", lang); } catch (_) {}
  }
  try { document.documentElement.lang = lang === "en" ? "en" : "zh-CN"; } catch (_) {}
}

export function getLanguage() {
  return lang;
}

// 检测系统语言：仅用于「用户从未显式选择」时的默认值。
// WebView（WebView2 / WKWebView）的 navigator.language 跟随操作系统显示语言。
/** @returns {"zh" | "en"} */
export function detectSystemLanguage() {
  try {
    const navLang = String(navigator.language || (navigator.languages && navigator.languages[0]) || "").toLowerCase();
    if (navLang.startsWith("zh")) return "zh";
    return "en";
  } catch (_) {
    return "zh";
  }
}

// 同步语言优先级：显式设置（Settings.language）> localStorage > 系统语言检测。
// 检测结果不写入 localStorage：用户从未显式选择时，每次启动跟随系统语言。
export function syncLanguageFromSettings(settings) {
  let l = null;
  if (settings && settings.language) l = settings.language;
  if (!l) { try { l = localStorage.getItem("adm_lang"); } catch (_) {} }
  if (!l) { setLanguage(detectSystemLanguage(), false); return; }
  setLanguage(l);
}

/** @param {string} str @returns {string} */
export function t(str) {
  if (lang === "zh") return str;
  if (str == null) return str;
  const v = EN[str];
  return v !== undefined ? v : str;
}

// 带变量替换：tV("下载失败: {err}", { err: e })
/** @param {string} str @param {Object<string,string|number>|undefined} vars @returns {string} */
export function tV(str, vars) {
  let out = t(str);
  if (vars) {
    for (const k of Object.keys(vars)) {
      out = out.replace(new RegExp("\\{" + k + "\\}", "g"), String(vars[k]));
    }
  }
  return out;
}

// 将根节点下所有带 data-i18n 的元素文本替换为当前语言
export function applyToDOM(root) {
  if (!root) return;
  const nodes = root.querySelectorAll ? root.querySelectorAll("[data-i18n]") : [];
  nodes.forEach((el) => {
    const key = el.getAttribute("data-i18n");
    if (key) el.textContent = t(key);
  });
  const placeholders = root.querySelectorAll ? root.querySelectorAll("[data-i18n-placeholder]") : [];
  placeholders.forEach((el) => {
    const key = el.getAttribute("data-i18n-placeholder");
    if (key) el.setAttribute("placeholder", t(key));
  });
}

// 挂到全局，供 index.html 壳层（非 ESM 上下文）使用
if (typeof window !== "undefined") {
  // 类型注释放宽：window.ADM 的形状由 types.d.ts 声明（含 showUpdateDialog 等），
  // 这里只合并注入 i18n，运行时由 index.html 先定义 ADM 再被本模块增强
  window.ADM = /** @type {any} */ (Object.assign(window.ADM || {}, {
    i18n: { t, tV, setLanguage, getLanguage, syncLanguageFromSettings, applyToDOM },
  }));
}

export default { t, tV, setLanguage, getLanguage, syncLanguageFromSettings, applyToDOM };
