// 「视频生成」页：ComfyUI（MiniMax-H3）服务入口——启动/停止/重启/状态/日志/首次指引。
// 约定（见 AGENTS.md）：视图模块只做本页 DOM 与交互；事件监听必须在壳层注册（这里只实现
// handleTauriEvent 做 DOM 更新）；服务生命周期由后端持有，切页/关闭窗口不影响运行中的容器。
import { t as _t } from "../i18n.js";

const template = `
<style>
  #video-root {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 20px;
    gap: 14px;
    overflow-y: auto;
  }

  .video-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }

  .video-title {
    font-size: 18px;
    font-weight: 600;
    color: var(--c-text-hi);
  }

  .video-sub {
    font-size: 12px;
    color: var(--c-text-2);
  }

  .video-card {
    background: var(--c-panel);
    border: 1px solid var(--c-border);
    border-radius: 10px;
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .video-card-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--c-text-hi);
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .video-row {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
  }

  .video-label {
    width: 56px;
    flex-shrink: 0;
    color: var(--c-text-2);
  }

  .video-value {
    color: var(--c-text-hi);
    word-break: break-all;
  }

  .video-state-dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--c-border-hi);
    margin-right: 6px;
  }

  .video-state-dot.running { background: #3fb950; }
  .video-state-dot.starting { background: #d29922; }

  .video-port-input {
    width: 88px;
    background: var(--c-raise);
    color: var(--c-text-hi);
    border: 1px solid var(--c-border-hi);
    border-radius: 6px;
    padding: 3px 8px;
    font-size: 13px;
  }

  .video-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }

  .video-btn {
    background: var(--c-raise);
    color: var(--c-text-hi);
    border: 1px solid var(--c-border-hi);
    border-radius: 8px;
    padding: 7px 14px;
    font-size: 13px;
    cursor: pointer;
    transition: background 0.15s, opacity 0.15s;
  }

  .video-btn:hover:not([disabled]) { background: var(--c-raise-2); }
  .video-btn[disabled] { opacity: 0.45; cursor: not-allowed; }
  .video-btn-primary { background: var(--c-accent); border-color: var(--c-accent); color: #fff; }
  .video-btn-primary:hover:not([disabled]) { background: #7d75ff; }
  .video-btn-ghost { background: transparent; }
  .video-btn-sm { padding: 3px 10px; font-size: 12px; margin-left: auto; }

  .video-guide {
    margin: 0;
    padding-left: 20px;
    font-size: 13px;
    color: var(--c-text);
    line-height: 1.7;
  }

  .video-mounts {
    font-size: 12px;
    color: var(--c-text-2);
    line-height: 1.6;
    word-break: break-all;
  }

  .video-warn {
    border-color: #d29922;
    background: rgba(210, 153, 34, 0.08);
    flex-direction: row;
    align-items: center;
    gap: 12px;
  }

  .video-warn-text {
    font-size: 13px;
    color: #e3b341;
    flex: 1;
  }

  .video-inline-btn { margin-left: auto; }

  .video-head-btn { margin-left: auto; }

  .video-progress {
    height: 6px;
    background: var(--c-raise);
    border-radius: 4px;
    overflow: hidden;
  }

  .video-progress-fill {
    height: 100%;
    width: 0%;
    background: var(--c-accent);
    transition: width 0.2s;
  }

  .video-toast {
    position: fixed;
    left: 50%;
    bottom: 96px;
    transform: translateX(-50%);
    background: var(--c-raise-2);
    color: var(--c-text-hi);
    border: 1px solid var(--c-border-hi);
    border-radius: 8px;
    padding: 8px 16px;
    font-size: 13px;
    z-index: 50;
    display: none;
  }
</style>
<div id="video-root">
  <div class="video-head">
    <div class="video-title">${_t("视频生成")}</div>
    <div class="video-sub">MiniMax-H3 · ComfyUI · NVFP4</div>
    <button id="video-refresh" class="video-btn video-btn-ghost video-btn-sm video-head-btn">${_t("刷新状态")}</button>
  </div>

  <div class="video-card">
    <div class="video-row">
      <span class="video-label">${_t("状态")}</span>
      <span class="video-value"><span class="video-state-dot" id="video-state-dot"></span><span id="video-state">--</span></span>
    </div>
    <div class="video-row">
      <span class="video-label">${_t("地址")}</span>
      <span class="video-value" id="video-url">--</span>
    </div>
    <div class="video-row">
      <span class="video-label">${_t("端口")}</span>
      <span class="video-value">
        <input id="video-port" class="video-port-input" type="number" min="1024" max="65535" />
        <button id="video-port-save" class="video-btn video-btn-ghost video-btn-sm">${_t("保存")}</button>
      </span>
    </div>
    <div class="video-row">
      <span class="video-label">${_t("镜像")}</span>
      <span class="video-value" id="video-image">--</span>
    </div>
    <div class="video-row">
      <span class="video-label">${_t("容器")}</span>
      <span class="video-value" id="video-container">--</span>
    </div>
    <div class="video-row">
      <span class="video-label">${_t("挂载")}</span>
      <span class="video-mounts" id="video-mounts">--</span>
    </div>
  </div>

  <div class="video-card">
    <div class="video-card-title">${_t("环境准备（首次使用）")}</div>
    <div class="video-row">
      <span class="video-label">${_t("镜像")}</span>
      <span class="video-value" id="video-image-state">--</span>
      <button id="video-pull-image" class="video-btn video-btn-ghost video-btn-sm video-inline-btn">${_t("下载镜像")}</button>
    </div>
    <div class="video-row">
      <span class="video-label">${_t("权重")}</span>
      <span class="video-value" id="video-weights-state">--</span>
      <button id="video-download-weights" class="video-btn video-btn-ghost video-btn-sm video-inline-btn">${_t("下载权重")}</button>
    </div>
    <div class="video-progress" id="video-progress-wrap" style="display:none;">
      <div class="video-progress-fill" id="video-progress-bar"></div>
    </div>
    <div class="video-row">
      <span class="video-label">${_t("日志")}</span>
      <span class="video-value" id="video-log-hint">${_t("运行日志按统一格式写入本地文件（设置 → 运行日志 可查看）")}</span>
      <button id="video-open-logs" class="video-btn video-btn-ghost video-btn-sm video-inline-btn">${_t("打开日志目录")}</button>
    </div>
    <div class="video-mounts" id="video-setup-paths">--</div>
  </div>

  <div class="video-actions">
    <button id="video-start" class="video-btn video-btn-primary">${_t("启动 ComfyUI")}</button>
    <button id="video-stop" class="video-btn">${_t("停止")}</button>
    <button id="video-restart" class="video-btn">${_t("重启")}</button>
    <button id="video-open" class="video-btn">${_t("打开 WebUI")}</button>
    <button id="video-copy-output" class="video-btn video-btn-ghost">${_t("复制产物路径")}</button>
  </div>

  <div class="video-card video-warn" id="video-exclusive" style="display:none;">
    <div class="video-warn-text" id="video-exclusive-text"></div>
    <button id="video-exclusive-btn" class="video-btn video-btn-primary">${_t("停止当前模型并启动 ComfyUI")}</button>
  </div>

  <div class="video-card">
    <div class="video-card-title">${_t("首次使用指引")}</div>
    <ol class="video-guide">
      <li>${_t("点击「打开 WebUI」→ 模板库 → Video → MiniMax H3 T2V / I2V / R2V")}</li>
      <li>${_t("跟随弹窗下载权重（NVFP4-AWQ 编码器 + 量化 DiT，约 44 GB，仅一次）")}</li>
      <li>${_t("首次建议 480P / 5 秒跑通；低延迟可接 turbo 8step LoRA（步数 8）")}</li>
      <li>${_t("与 SGLang 模型互斥：128 GB 统一内存不能同时驻留两套权重")}</li>
    </ol>
  </div>

  <div class="video-toast" id="video-toast"></div>
</div>
`;

const invoke = () => window.__adm_invoke;
const S = () => window.__adm_state;

const PORT_KEY = "adm_comfyui_port";
const DEFAULT_PORT = 8188;
/** 环境就绪状态（镜像 / 权重），由后端 comfyui_setup_status 返回 */
let setup = /** @type {any} */ ({ image_exists: false, weights_downloaded: false, weights_partial: false, weights_bytes: 0, weights_total_bytes: 0, weights_missing: [], weights_missing_count: 0, weights_dir: "" });
/** 视图是否已挂载：拉镜/权重下载等长任务在切页后完成时不再回写已移除的 DOM */
let mounted = false;
/** 停止（docker stop/rm）进行中：按钮/状态卡显示「正在停止中...」，避免误以为没反应 */
let stopping = false;
/** 镜像下载（docker pull）进行中：进度取 st.pullProgress[modelId]（model-pull-progress 事件） */
let pullingImage = false;
/** 权重下载速度（bytes/s，来自 download-progress 事件，仅用于本页展示） */
let downloadSpeed = 0;
/** @type {number | null} */
let toastTimer = null;

/** @param {string} id */
function el(id) {
  return /** @type {any} */ (document.getElementById(id));
}

/** ComfyUI 配置条目（远程 model.json 中 engine === "comfyui"） */
function entry() {
  const list = S().modelList || [];
  return list.find(function (m) {
    return String(m.engine || "").toLowerCase() === "comfyui";
  }) || null;
}

function getPort() {
  const raw = parseInt(String(localStorage.getItem(PORT_KEY) || ""), 10);
  return raw >= 1024 && raw <= 65535 ? raw : DEFAULT_PORT;
}

/** @param {number} p */
function setPort(p) {
  localStorage.setItem(PORT_KEY, String(p));
}

function webuiUrl() {
  return "http://127.0.0.1:" + getPort();
}

/** @param {string} entryStr 形如 host:container[:ro|:rw] */
function parseMount(entryStr) {
  let s = String(entryStr || "").trim();
  let readOnly = false;
  if (s.endsWith(":ro") || s.endsWith(":rw")) {
    readOnly = s.endsWith(":ro");
    s = s.slice(0, -3);
  }
  const idx = s.indexOf(":/");
  if (idx < 0) return null;
  return { host: s.slice(0, idx), container: s.slice(idx + 1), readOnly: readOnly };
}

/** 产物目录的宿主路径（取自挂载映射 <host>:…:/opt/ComfyUI/output） */
function hostOutputDir() {
  const e = entry();
  const mounts = (e && e.vllm_extra_mounts) || [];
  for (let i = 0; i < mounts.length; i++) {
    const m = parseMount(mounts[i]);
    if (m && m.container === "/opt/ComfyUI/output") return m.host;
  }
  return "";
}

/** @param {string} msg */
function notify(msg) {
  const t = el("video-toast");
  if (!t) return;
  t.textContent = msg;
  t.style.display = "block";
  if (toastTimer !== null) clearTimeout(toastTimer);
  toastTimer = setTimeout(function () { t.style.display = "none"; }, 3200);
}

function render() {
  if (!mounted) return;
  const e = entry();
  const st = S();
  const modelId = e ? e.model_id : null;
  const running = !!(modelId && st.runningModelId === modelId);
  const pull = modelId && st.pullProgress ? st.pullProgress[modelId] : undefined;
  // 拉镜（model-pull-progress）不再置 startingModelId（见 index.html），故这里只需看启动流程自身
  const starting = !!(modelId && st.startingModelId === modelId);

  const dot = el("video-state-dot");
  const stateText = el("video-state");
  if (!e) {
    dot.className = "video-state-dot";
    stateText.textContent = _t("未配置 ComfyUI 条目（检查远程 model.json）");
  } else if (stopping) {
    // 停止可能持续数秒（docker stop → rm），先明确告知正在停止
    dot.className = "video-state-dot starting";
    stateText.textContent = _t("正在停止中...");
  } else if (running) {
    dot.className = "video-state-dot running";
    stateText.textContent = _t("运行中");
  } else if (starting) {
    dot.className = "video-state-dot starting";
    stateText.textContent = _t("启动中...") + (pull !== undefined && pull > 0 ? " " + pull + "%" : "");
  } else {
    dot.className = "video-state-dot";
    stateText.textContent = _t("未启动");
  }

  el("video-url").textContent = running && !stopping ? webuiUrl() : "--";
  const portInput = el("video-port");
  if (portInput && document.activeElement !== portInput) portInput.value = String(getPort());
  el("video-image").textContent = (e && (e.engine_image || e.vllm_image)) || "--";
  el("video-container").textContent = modelId ? "adm-comfyui-" + modelId : "--";

  const mounts = (e && e.vllm_extra_mounts) || [];
  el("video-mounts").textContent = mounts.length
    ? mounts.map(function (s) {
        const m = parseMount(s);
        return m ? m.host + " → " + m.container + (m.readOnly ? " (ro)" : "") : String(s);
      }).join("\n")
    : _t("未配置附加挂载（权重与产物将落在容器内）");

  // 环境准备状态（镜像 / 权重）
  const dlProgress = modelId && st.downloadingModels ? st.downloadingModels[modelId] : undefined;
  const downloading = dlProgress !== undefined;
  const imageState = el("video-image-state");
  const weightsState = el("video-weights-state");
  const buildBtn = el("video-pull-image");
  const weightsBtn = el("video-download-weights");
  const progressWrap = el("video-progress-wrap");
  if (!e) {
    imageState.textContent = "--";
    weightsState.textContent = "--";
    buildBtn.disabled = true;
    weightsBtn.disabled = true;
    progressWrap.style.display = "none";
  } else {
    if (pullingImage) {
      // 拉镜进度（model-pull-progress 事件）：与权重下载共用底部进度条
      const imagePct = pull !== undefined ? pull : 0;
      imageState.textContent = _t("下载中...") + " " + imagePct + "%";
      progressWrap.style.display = "block";
      el("video-progress-bar").style.width = imagePct + "%";
    } else {
      imageState.textContent = setup.image_exists ? _t("已下载") : _t("未下载");
    }
    buildBtn.disabled = pullingImage || !!setup.image_exists || downloading;
    buildBtn.textContent = setup.image_exists ? _t("已下载") : (pullingImage ? _t("下载中...") : _t("下载镜像"));
    buildBtn.title = downloading ? _t("权重下载中，请等下载完成后再下载镜像") : "";
    if (downloading) {
      const speed = formatSpeed(downloadSpeed);
      weightsState.textContent = _t("下载中") + " " + dlProgress + "%" + (speed ? " · " + speed : "");
      weightsBtn.textContent = _t("停止下载");
      weightsBtn.disabled = false;
      progressWrap.style.display = "block";
      el("video-progress-bar").style.width = dlProgress + "%";
    } else {
      if (setup.weights_downloaded) {
        weightsState.textContent = _t("已下载") + "（" + formatBytes(setup.weights_bytes) + "）";
        weightsBtn.textContent = _t("已下载");
        weightsBtn.disabled = true;
      } else {
        // 后端按下载清单逐文件校验：有缺文件时给出「已下载 / 共 / 缺失个数」，tooltip 列具体文件名
        const missing = Number(setup.weights_missing_count) || 0;
        const missingList = Array.isArray(setup.weights_missing) ? setup.weights_missing : [];
        const sizeText = formatBytes(setup.weights_bytes) +
          (setup.weights_total_bytes ? " / " + formatBytes(setup.weights_total_bytes) : "");
        weightsState.textContent = setup.weights_partial
          ? _t("已中断") + "（" + sizeText +
            (missing ? " · " + _t("缺失") + " " + missing + _t(" 个文件") : "") +
            _t("，可继续断点续传）")
          : _t("未下载") + _t("（首次约 67 GB）");
        const tip = missingList.length
          ? _t("缺失文件") + "（" +
            (missing > missingList.length ? missingList.length + "/" + missing : String(missingList.length)) +
            "）：\n" + missingList.join("\n")
          : "";
        weightsState.title = tip;
        weightsBtn.title = pullingImage ? _t("镜像下载中，请等下载完成后再下载权重") : tip;
        weightsBtn.textContent = setup.weights_partial ? _t("继续下载") : _t("下载权重");
        weightsBtn.disabled = pullingImage;
      }
      // 拉镜进行中时保留进度条（与上面的拉镜百分比共用）
      if (!pullingImage) progressWrap.style.display = "none";
    }
    el("video-setup-paths").textContent = _t("权重目录") + ": " + (setup.weights_dir || "--");
  }

  el("video-start").disabled = !e || running || starting || stopping || pullingImage || !setup.image_exists;
  el("video-start").title = !setup.image_exists ? _t("需先下载镜像") : "";
  const stopBtn = el("video-stop");
  stopBtn.disabled = !running || stopping;
  stopBtn.textContent = stopping ? _t("停止中...") : _t("停止");
  el("video-restart").disabled = !running || stopping;
  el("video-open").disabled = !running || stopping;

  // 互斥提示：其它模型（SGLang 等）正在运行时
  const other = st.runningModelId && modelId && st.runningModelId !== modelId ? String(st.runningModelId) : "";
  const warn = el("video-exclusive");
  if (other) {
    warn.style.display = "flex";
    el("video-exclusive-text").textContent = _t("当前有模型正在运行（") + other + _t("），128 GB 统一内存不能同时驻留两套 H3 权重，请先停止它");
  } else {
    warn.style.display = "none";
  }
}

/** @param {number} bytes */
function formatBytes(bytes) {
  if (!bytes || bytes <= 0) return "0 GB";
  return (bytes / 1e9).toFixed(1) + " GB";
}

/** 下载速度（bytes/s → 人类可读；未知/0 返回空串） @param {number} bytesPerSec */
function formatSpeed(bytesPerSec) {
  if (!bytesPerSec || bytesPerSec <= 0) return "";
  return bytesPerSec >= 1e9
    ? (bytesPerSec / 1e9).toFixed(1) + " GB/s"
    : Math.round(bytesPerSec / 1e6) + " MB/s";
}

/** 查询镜像/权重就绪状态（后端 comfyui_setup_status） */
async function refreshSetup() {
  const e = entry();
  if (!e) return;
  const image = e.engine_image || e.vllm_image || "";
  try {
    // 传下载清单：后端逐文件校验权重完整性（缺失时能列出具体文件名）
    setup = await invoke()("comfyui_setup_status", { modelId: e.model_id, image: image, modelFiles: e.model_download_files || [] });
  } catch (err) {
    console.warn("[video] 查询 ComfyUI 环境状态失败:", err);
  }
}

/** 该模型权重是否正在下载（download-progress / download-complete 事件维护 st.downloadingModels） */
function isWeightsDownloading(modelId) {
  const dl = S().downloadingModels || {};
  return !!modelId && dl[modelId] !== undefined;
}

/** 下载镜像（后端 pull_comfyui_image；进度走 model-pull-progress 事件） */
async function handlePullImage() {
  const e = entry();
  if (!e || pullingImage) return;
  // 镜像与权重不允许一起下载（后端同样拒绝，这里提前提示并保持按钮禁用）
  if (isWeightsDownloading(e.model_id)) {
    notify(_t("权重下载中，请等下载完成后再下载镜像"));
    return;
  }
  const image = e.engine_image || e.vllm_image || "";
  if (!image) { notify(_t("未配置 engine_image")); return; }
  pullingImage = true;
  render();
  try {
    await invoke()("pull_comfyui_image", { modelId: e.model_id, image: image });
    notify(_t("镜像下载完成"));
  } catch (err) {
    notify(_t("镜像下载失败: ") + err);
  }
  pullingImage = false;
  // 拉镜进度只用于本页展示，清掉避免下次拉镜沿用旧百分比
  if (S().pullProgress) delete S().pullProgress[e.model_id];
  await refreshSetup();
  render();
}

/** 下载 ComfyUI 权重（复用模型下载链路：HF 仓库 ID + include 过滤 + 断点续传 + .done） */
async function handleDownloadWeights() {
  const e = entry();
  if (!e) return;
  // 镜像与权重不允许一起下载（后端同样拒绝，这里提前提示并保持按钮禁用）
  if (pullingImage) {
    notify(_t("镜像下载中，请等下载完成后再下载权重"));
    return;
  }
  const files = e.model_download_files || [];
  if (!files.length) { notify(_t("模型清单未配置 model_download_files")); return; }
  S().downloadingModels[e.model_id] = 0;
  render();
  try {
    await invoke()("download_model", {
      modelId: e.model_id,
      modelFiles: files,
      vllmImage: e.engine_image || e.vllm_image || null,
    });
  } catch (err) {
    delete S().downloadingModels[e.model_id];
    notify(_t("下载失败: ") + err);
  }
  await refreshSetup();
  render();
}

/** 停止权重下载（保留 .part 续传） */
async function handleStopDownload() {
  const e = entry();
  if (!e) return;
  try {
    await invoke()("cancel_download", { modelId: e.model_id });
    notify(_t("已请求停止下载"));
  } catch (err) {
    notify(_t("停止下载失败: ") + err);
  }
  render();
}

async function refreshModelList() {
  try {
    const list = await invoke()("fetch_model_list");
    if (Array.isArray(list) && list.length) S().modelList = list;
  } catch (err) {
    console.warn("[video] 拉取模型清单失败:", err);
  }
}

async function handleStart() {
  const e = entry();
  if (!e) { notify(_t("未配置 ComfyUI 条目（检查远程 model.json）")); return; }
  const st = S();
  if (st.runningModelId && st.runningModelId !== e.model_id) {
    notify(_t("请先停止当前模型再启动 ComfyUI"));
    return;
  }
  const port = getPort();
  st.startingModelId = e.model_id;
  render();
  try {
    await invoke()("start_model", {
      modelId: e.model_id,
      params: { ctx_size: null, port: port, host: null },
      device: st.currentDeviceFilter && st.currentDeviceFilter !== "all" ? st.currentDeviceFilter : null,
      vllmImage: e.vllm_image || null,
      vllmFlags: e.vllm_flags || null,
      vllmEnv: e.vllm_env || null,
      extraMounts: e.vllm_extra_mounts || null,
      engine: e.engine || null,
      engineImage: e.engine_image || null,
      engineCommand: e.engine_command || null,
      engineReadyPatterns: e.engine_ready_patterns || null,
      engineReadyProbe: !!e.engine_ready_probe,
      engineReadyPath: e.engine_ready_path || null,
    });
  } catch (err) {
    S().startingModelId = null;
    notify(_t("启动失败: ") + err);
  }
  render();
}

async function handleStop() {
  // 不依赖 entry()（远程清单拉不到时也要能停掉正在运行的容器）
  if (stopping) return;
  stopping = true;
  render();
  try {
    await invoke()("stop_model");
    S().runningModelId = null;
    S().runningModelPort = null;
  } catch (err) {
    notify(_t("停止失败: ") + err);
  }
  stopping = false;
  // 停止后容器/端口状态需重新探测（stop_model 内部是 docker stop + rm）
  await refreshSetup();
  render();
}

async function handleRestart() {
  await handleStop();
  await new Promise(function (resolve) { setTimeout(resolve, 1500); });
  await handleStart();
}

async function handleExclusiveStart() {
  await handleStop();
  await new Promise(function (resolve) { setTimeout(resolve, 1500); });
  await handleStart();
}

export default {
  template,

  /** @param {HTMLElement} root */
  mount(root) {
    root.innerHTML = template;
    mounted = true;

    el("video-start").addEventListener("click", function () { void handleStart(); });
    el("video-stop").addEventListener("click", function () { void handleStop(); });
    el("video-restart").addEventListener("click", function () { void handleRestart(); });
    el("video-exclusive-btn").addEventListener("click", function () { void handleExclusiveStart(); });
    el("video-open").addEventListener("click", function () { window.openUrl(webuiUrl()); });
    el("video-copy-output").addEventListener("click", function () {
      const dir = hostOutputDir();
      if (!dir) { notify(_t("未找到产物目录挂载")); return; }
      navigator.clipboard.writeText(dir);
      notify(_t("已复制: ") + dir);
    });
    el("video-refresh").addEventListener("click", function () {
      void (async function () {
        await refreshModelList();
        await refreshSetup();
        render();
        notify(_t("状态已刷新"));
      })();
    });
    el("video-pull-image").addEventListener("click", function () { void handlePullImage(); });
    el("video-download-weights").addEventListener("click", function () {
      const e = entry();
      if (!e) return;
      const dl = S().downloadingModels ? S().downloadingModels[e.model_id] : undefined;
      if (dl !== undefined) { void handleStopDownload(); } else { void handleDownloadWeights(); }
    });
    el("video-open-logs").addEventListener("click", function () {
      invoke()("open_log_dir").catch(function (err) { notify(_t("打开日志目录失败: ") + err); });
    });
    el("video-port-save").addEventListener("click", function () {
      const raw = parseInt(String(el("video-port").value || ""), 10);
      if (!(raw >= 1024 && raw <= 65535)) { notify(_t("端口需在 1024-65535 之间")); return; }
      setPort(raw);
      notify(_t("端口已保存，重启 ComfyUI 后生效"));
      render();
    });

    render();
    void (async function () {
      await refreshModelList();
      await refreshSetup();
      render();
    })();
  },

  unmount() {
    // 服务生命周期由后端持有：切页不停容器（日志已不在页面展示，写入统一本地日志文件）
    mounted = false;
  },

  /**
   * 壳层全局监听转发（只做 DOM 更新）
   * @param {string} type
   * @param {any} payload
   */
  handleTauriEvent(type, payload) {
    const p = payload || {};
    // model-log 事件不再由本页消费（日志栏已移除）：后端 logger::model_log 已落盘到统一日志文件
    if (type === "download-progress") {
      downloadSpeed = Number(p.speed) || 0;
      render();
      return;
    }
    if (type === "model-pull-progress" || type === "download-cancelled" || type === "download-error") {
      if (type !== "model-pull-progress") downloadSpeed = 0;
      render();
      return;
    }
    if (type === "download-complete") {
      downloadSpeed = 0;
      // 权重下完后的自动拉镜失败（后端 pull_image_if_configured 发 image-pull-failed）：页内提示
      if (p.type === "image-pull-failed") {
        notify(_t("镜像下载失败: ") + (p.error || p.image || ""));
      }
      void (async function () {
        await refreshSetup();
        render();
      })();
      return;
    }
    if (type === "model-started") {
      // 运行中也可能通过 ComfyUI 弹窗补下权重 → 顺带刷新环境状态
      void (async function () {
        await refreshSetup();
        render();
      })();
      return;
    }
    if (type === "model-stopped" || type === "model-error") {
      render();
    }
  },
};
