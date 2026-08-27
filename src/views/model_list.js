// @ts-nocheck -- 历史视图暂未类型化（jsconfig checkJs 全局开启，新代码请勿加此标记）
import { t as _t } from "../i18n.js";
const template = `
<style>
  /* 全局 reset（*）由 index.html 壳层统一提供，视图内不重复定义；选择器尽量限定在本视图容器内 */

  .page-title {
    font-size: 18px;
    font-weight: 600;
    color: var(--c-text-hi);
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
    padding: 20px 20px 16px;
  }

  .page-title::before {
    content: "";
    display: inline-block;
    width: 4px;
    height: 18px;
    background: var(--c-accent);
    border-radius: 2px;
  }

  #model-list-root {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .filter-bar {
    flex-shrink: 0;
  }

  main {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }

  .card-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 16px;
    margin: 12px 20px 20px;
  }

  .model-card {
    position: relative;
    background: var(--c-panel);
    border: 1px solid var(--c-border);
    border-radius: 10px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow: hidden;
    transition: border-color 0.2s, transform 0.2s, box-shadow 0.2s;
  }

  .model-card:hover {
    border-color: var(--c-accent);
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.3);
  }

  .model-card.card-running {
    border-color: rgba(33, 150, 243, 0.5);
    box-shadow: inset 3px 0 0 #2196f3;
  }

  .model-card.card-running:hover {
    box-shadow: inset 3px 0 0 #2196f3, 0 6px 20px rgba(0, 0, 0, 0.3);
  }

  .model-card.card-unavailable {
    opacity: 0.6;
  }

  .card-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 8px;
  }

  .card-header .status-badge {
    flex-shrink: 0;
  }

  .model-name {
    font-weight: 600;
    font-size: 15px;
    color: var(--c-text-hi);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .card-meta {
    font-size: 13px;
    color: var(--c-text-2);
  }

  .card-features {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .card-actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    border-top: 1px solid var(--c-border-soft);
    padding-top: 12px;
    margin-top: auto;
  }

  .card-progress {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 3px;
    background: rgba(var(--c-accent-rgb), 0.15);
  }

  .card-progress-fill {
    height: 100%;
    width: 0;
    background: var(--c-accent);
    transition: width 0.3s ease;
  }

  .grid-message {
    grid-column: 1 / -1;
    text-align: center;
    padding: 40px;
    color: var(--c-text-2);
  }

  .feature-badge {
    display: inline-block;
    padding: 3px 10px;
    border-radius: 12px;
    font-size: 12px;
    font-weight: 500;
  }

  .feature-supported {
    background: rgba(76, 175, 80, 0.15);
    color: #4caf50;
    border: 1px solid rgba(76, 175, 80, 0.3);
  }

  .status-badge {
    display: inline-block;
    padding: 3px 10px;
    border-radius: 12px;
    font-size: 12px;
    font-weight: 500;
  }

  .status-available {
    background: rgba(76, 175, 80, 0.15);
    color: #4caf50;
    border: 1px solid rgba(76, 175, 80, 0.3);
  }

  .status-unavailable {
    background: rgba(244, 67, 54, 0.15);
    color: #f44336;
    border: 1px solid rgba(244, 67, 54, 0.3);
  }

  .status-running {
    background: rgba(33, 150, 243, 0.15);
    color: #2196f3;
    border: 1px solid rgba(33, 150, 243, 0.3);
  }

  .btn {
    display: inline-block;
    padding: 5px 14px;
    border: none;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .btn-download {
    background: var(--c-accent);
    color: #fff;
  }

  .btn-download:hover:not(:disabled) {
    background: var(--c-accent-2);
  }

  .btn-download.downloaded {
    background: #2e7d32;
    cursor: default;
  }

  /* 下载中按钮可点击 = 停止下载 */
  .btn-cancel-download {
    background: #e53935;
  }

  .btn-cancel-download:hover:not(:disabled) {
    background: #c62828;
  }

  .btn-start {
    background: #1e88e5;
    color: #fff;
  }

  .btn-start:hover:not(:disabled) {
    background: #1565c0;
  }

  .btn-view {
    background: #00897b;
    color: #fff;
  }

  .btn-view:hover:not(:disabled) {
    background: #00695c;
  }

  .btn-stop {
    background: #e53935;
    color: #fff;
  }

  .btn-stop:hover:not(:disabled) {
    background: #c62828;
  }

  .btn-delete {
    background: transparent;
    color: #ef5350;
    border: 1px solid #ef5350;
  }

  .btn-delete:hover:not(:disabled) {
    background: #ef5350;
    color: #fff;
  }

  .modal-overlay {
    position: fixed;
    top: 0; left: 0; right: 0; bottom: 0;
    background: rgba(0,0,0,0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .modal-box {
    background: var(--c-panel);
    border-radius: 12px;
    padding: 24px;
    min-width: 360px;
    box-shadow: 0 8px 32px rgba(0,0,0,0.4);
  }

  .modal-box h3 {
    color: #fff;
    font-size: 16px;
    margin-bottom: 12px;
  }

  .modal-box p {
    color: var(--c-text-2);
    font-size: 14px;
    margin-bottom: 20px;
  }

  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .modal-actions .btn {
    padding: 8px 20px;
    font-size: 13px;
  }

  .btn-cancel {
    background: var(--c-border);
    color: var(--c-text);
  }

  .btn-cancel:hover {
    background: var(--c-border-hi);
  }

  .btn-confirm-delete {
    background: #ef5350;
    color: #fff;
  }

  .btn-confirm-delete:hover {
    background: #d32f2f;
  }

  .empty-state {
    grid-column: 1 / -1;
    text-align: center;
    padding: 60px 20px;
    color: var(--c-text-4);
  }

  .empty-state p {
    font-size: 14px;
  }

  .loading-spinner {
    display: inline-block;
    width: 16px;
    height: 16px;
    border: 2px solid rgba(var(--c-accent-rgb), 0.3);
    border-top-color: var(--c-accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    margin-right: 8px;
    vertical-align: middle;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .error-toast {
    position: fixed;
    top: 20px;
    right: 20px;
    background: #c62828;
    color: #fff;
    padding: 12px 20px;
    border-radius: 8px;
    font-size: 13px;
    z-index: 1000;
    animation: slideIn 0.3s ease;
    max-width: 400px;
  }

  @keyframes slideIn {
    from { transform: translateX(100%); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }

  .log-line {
    white-space: pre-wrap;
    word-break: break-all;
  }

  .log-line.error {
    color: #ff6b6b;
  }

  .log-line.success {
    color: #69db7c;
  }

  .log-line.info {
    color: #74c0fc;
  }

  .log-line.warning {
    color: #ffd43b;
  }

  .log-line.stderr {
    color: #ff8787;
  }

  .filter-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 20px 12px;
    flex-shrink: 0;
  }

  .filter-bar label {
    font-size: 13px;
    color: var(--c-text-2);
    white-space: nowrap;
  }

  .filter-bar select {
    background: var(--c-panel);
    color: var(--c-text);
    border: 1px solid var(--c-border);
    border-radius: 6px;
    padding: 6px 12px;
    font-size: 13px;
    outline: none;
    cursor: pointer;
    min-width: 160px;
  }

  .filter-bar select:focus {
    border-color: var(--c-accent);
  }

  .filter-bar .model-desc-text {
    display: none;
  }
</style>
<div id="model-list-root">
<div class="page-title">${_t("模型列表")}</div>
<div class="filter-bar">
  <label for="device-select">${_t("适配机型")}</label>
  <select id="device-select"></select>
</div>
<main>
  <div class="card-grid" id="model-grid">
    <div class="grid-message">
      <span class="loading-spinner"></span>${_t("正在加载模型列表...")}
    </div>
  </div>
</main>
<div id="delete-modal" class="modal-overlay" style="display:none;">
  <div class="modal-box">
    <h3>${_t("确认删除")}</h3>
    <p id="delete-modal-msg">${_t("确定要删除此模型吗？删除后无法恢复。")}</p>
    <div class="modal-actions">
      <button class="btn btn-cancel" id="delete-modal-cancel">${_t("取消")}</button>
      <button class="btn btn-confirm-delete" id="delete-modal-confirm">${_t("确认删除")}</button>
    </div>
  </div>
</div>
</div>
`;

let unlisteners = []; // kept for compat, no longer used (events handled globally)

// 模型日志批量写入（避免每行 stdout 一次 invoke）
let pendingLogLines = [];
let logFlushTimer = null;
function flushLogLines() {
  if (pendingLogLines.length === 0) { logFlushTimer = null; return; }
  var batch = pendingLogLines.splice(0);
  logFlushTimer = null;
  batch.forEach(function(item) {
    try { invoke()("write_app_log", { level: item.level, tag: "MODEL", message: item.line }); } catch (_) {}
  });
}

const invoke = () => window.__adm_invoke;
const listen = () => window.__adm_listen;
const S = () => window.__adm_state;

function formatBytes(bytes) {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + sizes[i];
}

function getUrlFilename(url) {
  return url ? url.split('/').pop() : null;
}

// HTML 转义：远端 model.json 内容会拼进 innerHTML / 属性，必须真实转义防注入
function escapeHtml(s) {
  if (s == null) return "";
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function isModelAvailable(model) {
  const dev = S().currentDeviceFilter;
  if (!dev || dev === "all") return true;
  const devices = model.model_support_devices || [];
  return devices.length === 0 || devices.includes(dev);
}

function isModelDownloaded(modelId) {
  const local = S().localModels.find(m => m.model_id === modelId);
  if (!local) return false;
  const model = S().modelList.find(m => m.model_id === modelId);
  // 新格式（多文件目录模型）：所有文件齐全才算下载完成
  if (model && model.model_download_files && model.model_download_files.length > 0) {
    return model.model_download_files.every(function(url) {
      const fname = getUrlFilename(url);
      return fname && local.files.includes(fname);
    });
  }
  return true;
}

function showToast(message) {
  const existing = document.querySelector(".error-toast");
  if (existing) existing.remove();
  const toast = document.createElement("div");
  toast.className = "error-toast";
  toast.textContent = message;
  document.body.appendChild(toast);
  setTimeout(() => toast.remove(), 10000);
}

function getFilteredModelList() {
  let list = S().modelList;
  // 机型过滤：model_support_devices 为空 = 全机型可用
  const dev = S().currentDeviceFilter;
  if (dev && dev !== "all") {
    list = list.filter(function(m) {
      const devices = m.model_support_devices || [];
      return devices.length === 0 || devices.includes(dev);
    });
  }
  return list;
}

async function populateDeviceFilter() {
  var select = document.getElementById("device-select");
  select.innerHTML = "";

  // 从远端模型配置收集机型集合（保持出现顺序去重），保证新机型接入后自动出现
  const devices = [];
  (S().modelList || []).forEach(function(m) {
    (m.model_support_devices || []).forEach(function(d) {
      if (!devices.includes(d)) devices.push(d);
    });
  });

  var optAll = document.createElement("option");
  optAll.value = "all";
  optAll.textContent = _t("全部机型");
  select.appendChild(optAll);
  devices.forEach(function(d) {
    var opt = document.createElement("option");
    opt.value = d;
    opt.textContent = d;
    select.appendChild(opt);
  });

  // 恢复上次选择（localStorage 记住，仅当机型仍存在时有效）
  var saved = "all";
  try { saved = localStorage.getItem("adm_device_filter") || "all"; } catch (_) {}
  if (saved !== "all" && !devices.includes(saved)) saved = "all";
  select.value = saved;
  S().currentDeviceFilter = saved;

  select.addEventListener("change", function() {
    S().currentDeviceFilter = this.value;
    try { localStorage.setItem("adm_device_filter", this.value); } catch (_) {}
    renderModelTable();
  });
}

function renderModelTable() {
  const grid = document.getElementById("model-grid");
  const filteredList = getFilteredModelList();
  const st = S();

  if (filteredList.length === 0) {
    grid.innerHTML = '<div class="empty-state"><p>' + _t("暂无可用模型") + '</p></div>';
    return;
  }

  grid.innerHTML = "";

  filteredList.forEach((model) => {
    const available = isModelAvailable(model);
    const downloaded = isModelDownloaded(model.model_id);
    const isRunning = st.runningModelId === model.model_id;
    const isStarting = st.startingModelId === model.model_id;

    const card = document.createElement("div");
    card.className = "model-card" + (isRunning ? " card-running" : (!available ? " card-unavailable" : ""));

    let statusHtml = "";
    if (isRunning) {
      statusHtml = '<span class="status-badge status-running">' + _t("已启动") + '</span>';
    } else if (isStarting) {
      statusHtml = '<span class="status-badge status-running">' + _t("启动中") + '</span>';
    } else if (available) {
      statusHtml = '<span class="status-badge status-available">' + _t("可用") + '</span>';
    } else {
      statusHtml = '<span class="status-badge status-unavailable">' + _t("不可用") + '</span>';
    }

    const partSize = st.partFiles[model.model_id];
    const downloadingProgress = st.downloadingModels[model.model_id];
    const safeModelId = escapeHtml(model.model_id);
    const modelFilesAttr = model.model_download_files && model.model_download_files.length > 0
      ? ' data-model-files="' + escapeHtml(JSON.stringify(model.model_download_files)) + '"'
      : '';
    const modelImageAttr = model.vllm_image
      ? ' data-model-image="' + escapeHtml(model.vllm_image) + '"'
      : '';
    let downloadBtnHtml = "";
    if (downloaded) {
      downloadBtnHtml = '';
    } else if (downloadingProgress !== undefined) {
      downloadBtnHtml = '<button class="btn btn-download btn-cancel-download" data-model-id="' + safeModelId + '" data-cancel-btn="' + safeModelId + '" id="dl-' + safeModelId + '">' + _t("停止下载") + ' ' + downloadingProgress + '%</button>';
    } else if (partSize && partSize > 0) {
      downloadBtnHtml = '<button class="btn btn-download" data-model-id="' + safeModelId + '" data-model-url="' + escapeHtml(model.model_url) + '"' + modelFilesAttr + modelImageAttr + ' id="dl-' + safeModelId + '">' + _t("继续下载") + '</button>';
    } else if (available) {
      downloadBtnHtml = '<button class="btn btn-download" data-model-id="' + safeModelId + '" data-model-url="' + escapeHtml(model.model_url) + '"' + modelFilesAttr + modelImageAttr + ' id="dl-' + safeModelId + '">' + _t("下载") + '</button>';
    } else {
      downloadBtnHtml = '<button class="btn btn-download" disabled>' + _t("下载") + '</button>';
    }

    let actionsHtml = "";
    if (isRunning) {
actionsHtml = '<button class="btn btn-view" id="view-' + safeModelId + '">' + _t("查看模型") + '</button>';
      actionsHtml += '<button class="btn btn-stop" data-stop-btn="' + safeModelId + '" id="stop-' + safeModelId + '">' + _t("关闭模型") + '</button>';
    } else if (isStarting) {
      // 启动中：从全局 pullProgress 恢复拉取进度显示（切页回来不丢）
      var pullPct = st.pullProgress && st.pullProgress[model.model_id];
      var startBtnText = (pullPct !== undefined && pullPct > 0)
        ? _t("拉取镜像 ") + pullPct + "%"
        : _t("启动中...");
      actionsHtml = '<button class="btn btn-start" disabled id="start-' + safeModelId + '" data-pull-pct="' + (pullPct || 0) + '">' + startBtnText + '</button>';
    } else if (downloaded && available) {
      actionsHtml = '<button class="btn btn-start" data-start-btn="' + safeModelId + '" id="start-' + safeModelId + '">' + _t("启动") + '</button>';
    } else if (downloaded) {
      actionsHtml = '<button class="btn btn-start" disabled>' + _t("启动") + '</button>';
    } else {
      actionsHtml = '';
    }
    if (downloaded && !isRunning) {
      actionsHtml += '<button class="btn btn-delete" data-delete-btn="' + safeModelId + '">' + _t("删除") + '</button>';
    }

    const features = [];
    if (model.support_tools) features.push('<span class="feature-badge feature-supported">' + _t("工具调用") + '</span>');
    if (model.support_reasoning) features.push('<span class="feature-badge feature-supported">' + _t("推理") + '</span>');
    if (model.support_images) features.push('<span class="feature-badge feature-supported">' + _t("图片识别") + '</span>');
    const featuresHtml = features.length > 0 ? '<div class="card-features">' + features.join('') + '</div>' : '';

    const progressVisible = downloadingProgress !== undefined;
    const progressValue = downloadingProgress !== undefined ? downloadingProgress : 0;

    card.innerHTML =
      '<div class="card-header"><span class="model-name" title="' + safeModelId + '">' + escapeHtml(model.model_id) + '</span>' + statusHtml + '</div>' +
      '<div class="card-meta">' + escapeHtml(model.model_type || '-') + ' · ' + escapeHtml(model.model_size) + '</div>' +
      featuresHtml +
      '<div class="card-actions">' + downloadBtnHtml + actionsHtml + '</div>' +
      '<div class="card-progress" data-progress-wrap="' + safeModelId + '" style="display:' + (progressVisible ? 'block' : 'none') + ';">' +
        '<div class="card-progress-fill" data-progress-bar="' + safeModelId + '" style="width:' + progressValue + '%;"></div>' +
      '</div>';

    grid.appendChild(card);
  });

  bindRowEvents();
}

function bindRowEvents() {
  const st = S();
  const dlBtns = document.querySelectorAll('#model-grid .btn-download:not(.downloaded):not([disabled]):not(.btn-cancel-download)');
  dlBtns.forEach(function(btn) {
    btn.addEventListener('click', function() { handleDownload(btn); });
  });
  // 下载中按钮 → 点击停止下载
  const cancelBtns = document.querySelectorAll('#model-grid .btn-cancel-download');
  cancelBtns.forEach(function(btn) {
    btn.addEventListener('click', function() { handleCancelDownload(btn.dataset.cancelBtn); });
  });
  const startBtns = document.querySelectorAll('#model-grid .btn-start[data-start-btn]');
  startBtns.forEach(function(btn) {
    btn.addEventListener('click', function() { handleStart(btn); });
  });
  const stopBtns = document.querySelectorAll('#model-grid .btn-stop[data-stop-btn]');
  stopBtns.forEach(function(btn) {
    btn.addEventListener('click', function() { handleStop(btn); });
  });
  const viewBtns = document.querySelectorAll('#model-grid .btn-view');
  viewBtns.forEach(function(btn) {
    btn.addEventListener('click', function() {
      const modelId = btn.id.replace('view-', '');
      goModel(modelId);
    });
  });
  const deleteBtns = document.querySelectorAll('#model-grid .btn-delete[data-delete-btn]');
  deleteBtns.forEach(function(btn) {
    btn.addEventListener('click', function() {
      const modelId = btn.dataset.deleteBtn;
      showDeleteConfirm(modelId);
    });
  });
}

async function handleDownload(btn) {
  const modelId = btn.dataset.modelId;
  const modelUrl = btn.dataset.modelUrl;
  console.log("[model_list] 开始下载模型:", modelId, "URL:", modelUrl);
  const vllmImage = btn.dataset.modelImage || null;
  let modelFiles = null;
  if (btn.dataset.modelFiles) {
    try { modelFiles = JSON.parse(btn.dataset.modelFiles); } catch (_) { modelFiles = null; }
  }
  if (btn) {
    const hasPart = S().partFiles[modelId] && S().partFiles[modelId] > 0;
    btn.textContent = hasPart ? _t("继续下载中...") : "0%";
    btn.disabled = false;
    btn.classList.add("btn-cancel-download");
    btn.dataset.cancelBtn = modelId;
    btn.onclick = function() { handleCancelDownload(modelId); };
  }

  try {
    await invoke()("download_model", { modelId: modelId, modelUrl: modelUrl, modelFiles: modelFiles, vllmImage: vllmImage });
    console.log("[model_list] 下载模型 invoke 完成:", modelId);
  } catch (e) {
    console.error("[model_list] 下载失败:", e);
    showToast(_t("下载失败: ") + e);
    if (btn) {
      btn.textContent = _t("下载");
      btn.disabled = false;
      btn.classList.remove("btn-cancel-download");
      delete btn.dataset.cancelBtn;
      btn.onclick = null;
    }
  }
}

// 点击下载中按钮 → 停止下载（Rust 侧置取消标志，保留 .part 供续传；事件驱动 UI 刷新）
async function handleCancelDownload(modelId) {
  console.log("[model_list] 停止下载模型:", modelId);
  try {
    await invoke()("cancel_download", { modelId: modelId });
  } catch (e) {
    console.error("[model_list] 停止下载失败:", e);
    showToast(_t("停止下载失败: ") + e);
  }
}

// Docker 权限修复弹窗
function showDockerPermissionDialog() {
  // 移除已有弹窗
  var existing = document.getElementById("docker-perm-overlay");
  if (existing) existing.remove();

  var overlay = document.createElement("div");
  overlay.id = "docker-perm-overlay";
  overlay.style.cssText = "position:fixed;inset:0;background:rgba(0,0,0,0.6);z-index:9999;display:flex;justify-content:center;align-items:center;";
  overlay.innerHTML =
    '<div style="background:var(--c-panel);border:1px solid var(--c-border);border-radius:12px;padding:28px 32px;max-width:440px;width:90%;box-shadow:0 8px 32px rgba(0,0,0,0.5);text-align:center;">' +
      '<div style="font-size:36px;margin-bottom:12px;">🔑</div>' +
      '<div style="font-size:18px;font-weight:600;color:var(--c-text-hi);margin-bottom:12px;">' + _t("Docker 权限不足") + '</div>' +
      '<div id="docker-perm-msg" style="font-size:14px;color:var(--c-text-2);line-height:1.7;margin-bottom:24px;">' +
        _t("当前用户不在 docker 组，无法启动模型。点击下方按钮，系统会弹出密码框自动修复权限，修复后需重启 ADM-BE 生效。") +
      '</div>' +
      '<div style="display:flex;gap:12px;justify-content:center;">' +
        '<button id="docker-perm-fix-btn" style="background:var(--c-accent);color:#fff;border:none;padding:10px 28px;border-radius:8px;font-size:14px;font-weight:500;cursor:pointer;">' + _t("一键修复") + '</button>' +
        '<button id="docker-perm-cancel-btn" style="background:var(--c-overlay);color:var(--c-text);border:none;padding:10px 28px;border-radius:8px;font-size:14px;cursor:pointer;">' + _t("取消") + '</button>' +
      '</div>' +
      '<div id="docker-perm-status" style="margin-top:16px;font-size:13px;color:var(--c-text-3);"></div>' +
    '</div>';
  document.body.appendChild(overlay);

  overlay.querySelector("#docker-perm-cancel-btn").addEventListener("click", function() { overlay.remove(); });
  overlay.querySelector("#docker-perm-fix-btn").addEventListener("click", async function() {
    var statusEl = overlay.querySelector("#docker-perm-status");
    var fixBtn = overlay.querySelector("#docker-perm-fix-btn");
    var msgEl = overlay.querySelector("#docker-perm-msg");
    fixBtn.disabled = true;
    fixBtn.textContent = _t("修复中...");
    statusEl.textContent = "";
    try {
      var msg = await invoke()("fix_docker_permission");
      if (msg === "PERMISSION_FIXED") {
        // 权限修复成功，系统正在注销登录
        msgEl.innerHTML = _t("权限修复成功，系统即将自动注销登录，重新登录后即可使用。");
        statusEl.style.color = "#4caf50";
        statusEl.textContent = "";
        fixBtn.style.display = "none";
        overlay.querySelector("#docker-perm-cancel-btn").textContent = _t("关闭");
      } else {
        statusEl.style.color = "#4caf50";
        statusEl.textContent = msg;
        fixBtn.textContent = _t("已修复，请重启");
        fixBtn.disabled = false;
        fixBtn.onclick = function() { overlay.remove(); };
      }
    } catch (err) {
      var errStr = String(err);
      if (errStr.indexOf("PKEXEC_CANCELLED") !== -1) {
        // 用户取消了密码框
        statusEl.style.color = "var(--c-text-3)";
        statusEl.textContent = _t("已取消，可重新点击修复");
        fixBtn.disabled = false;
        fixBtn.textContent = _t("一键修复");
      } else if (errStr.indexOf("FALLBACK_TERMINAL") !== -1) {
        // pkexec 不可用，切换为终端命令提示
        var parts = errStr.split("|");
        var user = parts[1] || "";
        var cmd = parts.slice(2).join("|");
        msgEl.innerHTML = _t("系统不支持自动修复，请在终端手动执行以下命令，然后重新登录后重启 ADM-BE：");
        statusEl.style.color = "var(--c-text-2)";
        statusEl.innerHTML = '<code style="display:block;background:var(--c-bg-deep);padding:12px 16px;border-radius:6px;font-family:monospace;font-size:13px;text-align:left;white-space:pre-wrap;margin-top:8px;">' + escapeHtml(cmd) + '</code>';
        fixBtn.style.display = "none";
      } else {
        statusEl.style.color = "#f44336";
        statusEl.textContent = _t("修复失败: ") + err;
        fixBtn.disabled = false;
        fixBtn.textContent = _t("一键修复");
      }
    }
  });
}

async function handleStart(btn) {
  const modelId = btn.dataset.startBtn;
  console.log("[model_list] 启动模型:", modelId);
  try {
    const settings = await invoke()("load_settings");
    const params = settings.launch_params || settings.launchParams;

    if (!params) {
      console.error("[DEBUG] params is undefined! settings keys:", Object.keys(settings));
    }

    const device = S().currentDeviceFilter && S().currentDeviceFilter !== "all" ? S().currentDeviceFilter : null;

    // 每次启动前重新拉取远程 model.json，保证 vllm_flags / vllm_image 用的是最新线上配置
    // （用户改远程清单后无需重启应用 / 重新进入列表页；拉取失败则沿用内存列表，不阻塞启动）
    try {
      const fresh = await invoke()("fetch_model_list");
      if (Array.isArray(fresh) && fresh.length) S().modelList = fresh;
    } catch (e) {
      console.warn("[model_list] 启动前刷新模型清单失败，沿用内存列表:", e);
    }

    // 模型配置的 vllm_image（完整镜像名）→ 指定该模型使用的镜像
    const model = (S().modelList || []).find(m => m.model_id === modelId);
    const vllmImage = (model && model.vllm_image) || null;
    // 模型配置的 vllm_flags（官方推荐启动参数）→ 优先级最高
    const vllmFlags = (model && model.vllm_flags) || null;
    // 模型配置的 vllm_env（官方推荐容器环境变量，KEY=VALUE）→ 注入 docker -e，优先级最高
    const vllmEnv = (model && model.vllm_env) || null;

    S().startingModelId = modelId;
    renderModelTable();

    await invoke()("start_model", { modelId: modelId, params: params, device: device, vllmImage: vllmImage, vllmFlags: vllmFlags, vllmEnv: vllmEnv });
    console.log("[model_list] 启动模型 invoke 完成:", modelId);
  } catch (e) {
    console.error("[model_list] 启动失败:", e);
    S().startingModelId = null;
    var errMsg = String(e);
    try { invoke()("write_app_log", { level: "ERROR", tag: "MODEL", message: "[启动失败] " + errMsg }); } catch (_) {}
    if (errMsg.indexOf("DOCKER_PERMISSION_DENIED") !== -1) {
      showDockerPermissionDialog();
    } else {
      showToast(_t("启动失败: ") + e);
    }
    renderModelTable();
  }
}

async function handleStop(btn) {
  console.log("[model_list] 停止模型");
  if (btn) {
    btn.disabled = true;
    btn.textContent = _t("停止中...");
  }
  try {
    await invoke()("stop_model");
    S().runningModelId = null;
    S().runningModelPort = null;
    renderModelTable();
  } catch (e) {
    showToast(_t("停止失败: ") + e);
    if (btn) { btn.disabled = false; btn.textContent = _t("关闭模型"); }
  }
}

function goModel(modelId) {
  const port = S().runningModelPort || 5678;
  // 直接用系统浏览器打开模型 WebUI（壳层 openUrl 走 opener 插件）
  window.openUrl("http://127.0.0.1:" + port);
}

function showDeleteConfirm(modelId) {
  const modal = document.getElementById("delete-modal");
  document.getElementById("delete-modal-msg").textContent = _t("确定要删除模型 \"") + modelId + _t("\" 吗？删除后无法恢复。");
  modal.style.display = "flex";
  modal.dataset.modelId = modelId;
}

function hideDeleteConfirm() {
  document.getElementById("delete-modal").style.display = "none";
}

async function handleDelete(modelId) {
  try {
    await invoke()("delete_local_model", { modelId: modelId });
    const idx = S().localModels.findIndex(function(m) { return m.model_id === modelId; });
    if (idx !== -1) S().localModels.splice(idx, 1);
    delete S().partFiles[modelId];
    renderModelTable();
  } catch (e) {
    showToast(_t("删除失败: ") + e);
  }
}

function updateProgressBar(modelId, progress) {
  const wrap = document.querySelector('[data-progress-wrap="' + modelId + '"]');
  if (wrap) wrap.style.display = "block";
  const bar = document.querySelector('[data-progress-bar="' + modelId + '"]');
  if (bar) bar.style.width = progress + "%";
}

// 字节/秒 → 可读速度：>=1MB/s 显示 MB/s，否则显示 KB/s（如 100KB/s）
function formatSpeed(bps) {
  if (!bps || bps <= 0) return "";
  const units = ["B/s", "KB/s", "MB/s", "GB/s"];
  let v = bps;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) { v /= 1024; u++; }
  let s = v >= 100 ? v.toFixed(0) : v.toFixed(1);
  if (s.endsWith(".0")) s = s.slice(0, -2);
  return s + units[u];
}

async function handleTauriEvent(type, payload) {
  // 状态已在 index.html 全局监听中更新，这里只做 DOM 更新
  const st = S();
  const { model_id, progress, error, port } = payload || {};

  switch (type) {
    case "download-progress": {
      const btn = document.querySelector('[data-model-id="' + model_id + '"]');
      if (btn && btn.classList.contains("btn-cancel-download")) {
        const speedText = payload && payload.speed ? " · " + formatSpeed(payload.speed) : "";
        btn.textContent = _t("停止下载") + " " + progress + "%" + speedText;
      }
      updateProgressBar(model_id, progress);
      break;
    }
    case "download-cancelled": {
      // 重新扫描 .part 大小 → 按钮显示「继续下载」，保留断点
      try {
        const parts = await invoke()("scan_part_files");
        st.partFiles = {};
        for (const p of parts) st.partFiles[p.model_id] = p.existing_size;
      } catch (_) {}
      renderModelTable();
      showToast(_t("已停止下载"));
      break;
    }
    case "download-complete": {
      if (payload && payload.type === "image-pull-failed") {
        showToast(_t("镜像拉取失败：") + (payload.image || "") + _t("；请检查网络或手动 docker pull，启动前需先补拉镜像"), true);
      }
      renderModelTable();
      break;
    }
    case "download-error": {
      showToast(_t("下载失败 [") + model_id + _t("]: ") + error);
      renderModelTable();
      break;
    }
    case "model-pull-progress": {
      const startBtn = document.getElementById("start-" + model_id);
      if (startBtn) {
        const prev = parseInt(startBtn.dataset.pullPct || "-1", 10);
        if (prev >= 0 && progress < prev) {
          startBtn.textContent = _t("切换镜像源重试中...");
        } else {
          startBtn.textContent = _t("拉取镜像 ") + progress + "%";
        }
        startBtn.dataset.pullPct = progress;
      }
      updateProgressBar(model_id, progress);
      break;
    }
    case "model-log": {
      if (payload && payload.line) {
        pendingLogLines.push({ level: payload.source === "stderr" ? "WARN" : "INFO", line: payload.line });
        if (!logFlushTimer) {
          logFlushTimer = setTimeout(flushLogLines, 500);
        }
      }
      break;
    }
    case "model-started":
    case "model-stopped": {
      renderModelTable();
      break;
    }
    case "model-error": {
      showToast(_t("模型错误 [") + model_id + _t("]: ") + error);
      renderModelTable();
      break;
    }
  }
}

async function init() {
  console.log("[model_list] init() 开始");
  const st = S();
  if (!st.systemInfo) {
    try {
      st.systemInfo = await invoke()("get_system_info");
      try {
        const gpuInfo = await invoke()("plugin:hwinfo|get_gpu_info");
        if (gpuInfo && gpuInfo.vramMb) {
          st.systemInfo.total_vram = gpuInfo.vramMb * 1024 * 1024;
          st.systemInfo.has_gpu = true;
        }
      } catch (_) {}
      try {
        const ramInfo = await invoke()("plugin:hwinfo|get_ram_info");
        if (ramInfo && ramInfo.sizeMb) {
          st.systemInfo.total_ram = ramInfo.sizeMb * 1024 * 1024;
        }
      } catch (_) {}
    } catch (e) {
      console.error("获取系统信息失败:", e);
    }
  }

  try { st.localModels = await invoke()("scan_local_models"); } catch (e) { console.error("扫描本地模型失败:", e); }

  try {
    const parts = await invoke()("scan_part_files");
    st.partFiles = {};
    for (const p of parts) st.partFiles[p.model_id] = p.existing_size;
  } catch (e) { console.error("扫描未完成下载失败:", e); }

  // 重新从后端同步下载状态（切页回来时恢复正在下载/已完成的进度）
  st.downloadingModels = {};
  try { st.downloadingModels = await invoke()("get_downloading_models"); } catch (e) { console.error("获取正在下载的模型失败:", e); }

  try {
    const status = await invoke()("get_model_status");
if (status.running) {
  st.runningModelId = status.model_id;
  st.runningModelPort = status.port;
}
  } catch (e) { console.error("获取模型状态失败:", e); }

  try {
    const list = await invoke()("fetch_model_list");
    st.modelList = list;
  } catch (e) {
    showToast(_t("获取模型列表失败: ") + e);
  }

  await populateDeviceFilter();
  renderModelTable();
  console.log("[model_list] init() 完成, 模型数量:", st.modelList.length);
}

/* setupListeners removed — events handled globally in index.html */

export default {
  template,
  handleTauriEvent: handleTauriEvent,
  mount(root) {
    console.log("[model_list] mount()");
    root.innerHTML = template;
  if (!S().currentDeviceFilter) S().currentDeviceFilter = "all";
  S().startingModelId = S().startingModelId || null;

    // 禁用页面右键（屏蔽浏览器默认菜单，删除弹窗在根容器内一并覆盖）
    var listRoot = document.getElementById("model-list-root");
    if (listRoot) listRoot.addEventListener("contextmenu", function(e) { e.preventDefault(); });

    init();

    document.getElementById("delete-modal-cancel").addEventListener("click", hideDeleteConfirm);
    document.getElementById("delete-modal-confirm").addEventListener("click", async function() {
      const modal = document.getElementById("delete-modal");
      const modelId = modal.dataset.modelId;
      hideDeleteConfirm();
      if (modelId) await handleDelete(modelId);
    });
    document.getElementById("delete-modal").addEventListener("click", function(e) {
      if (e.target === this) hideDeleteConfirm();
    });
  },
  unmount() {
    console.log("[model_list] unmount()");
  }
};
