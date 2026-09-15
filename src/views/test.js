// @ts-nocheck 历史视图暂未类型化（jsconfig checkJs 全局开启，新代码请勿加此标记）
import { t as _t } from "../i18n.js";

const template = `
<style>
  #test-root {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 20px;
  }

  .page-title {
    font-size: 18px;
    font-weight: 600;
    color: var(--c-text-hi);
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 16px;
    flex-shrink: 0;
  }

  .page-title::before {
    content: "";
    display: inline-block;
    width: 4px;
    height: 18px;
    background: var(--c-accent);
    border-radius: 2px;
  }

  #test-chat-pane {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    position: relative;
  }

  .test-chat-toolbar {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-bottom: 8px;
    flex-shrink: 0;
  }

  .test-chat-btn {
    background: var(--c-panel-2);
    border: 1px solid var(--c-border);
    color: var(--c-text-2);
    font-size: 12px;
    padding: 4px 12px;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s;
  }

  .test-chat-btn:hover {
    border-color: var(--c-accent);
    color: var(--c-text-hi);
  }

  #test-chat-frame {
    flex: 1;
    min-height: 0;
    width: 100%;
    border: 1px solid var(--c-border);
    border-radius: 8px;
    background: var(--c-panel);
  }

  #test-chat-hint {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    top: 34px;
    display: flex;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 0 24px;
    color: var(--c-text-4);
    font-size: 13px;
    background: var(--c-bg-deep);
    border: 1px solid var(--c-border);
    border-radius: 8px;
  }
</style>
<div id="test-root">
  <div class="page-title">${_t("模型测试")}</div>
  <div id="test-chat-pane">
    <div class="test-chat-toolbar">
      <button class="test-chat-btn" id="test-chat-reload">${_t("刷新")}</button>
      <button class="test-chat-btn" id="test-chat-open">${_t("在浏览器中打开")}</button>
    </div>
    <iframe id="test-chat-frame" allow="clipboard-read; clipboard-write; microphone"></iframe>
    <div id="test-chat-hint"></div>
  </div>
</div>
`;

const invoke = () => window.__adm_invoke;
const S = () => window.__adm_state;

// 已加载到 iframe 的模型 id（相同模型不重复重载）
let framedModelId = null;
// 缓存测试页地址（“在浏览器中打开”用）
let chatFrameUrl = "";
// 加载序号：模型快速启停时丢弃过期的异步结果，避免旧请求覆盖新状态
let loadSeq = 0;

function chatEls() {
  return {
    frame: document.getElementById("test-chat-frame"),
    hint: document.getElementById("test-chat-hint"),
    reload: document.getElementById("test-chat-reload"),
    open: document.getElementById("test-chat-open")
  };
}

/** 让 iframe 指向内置测试页（惰性启动 Rust 本地服务并把代理指向当前模型端口） */
async function loadChatFrame() {
  var seq = ++loadSeq;
  var els = chatEls();
  if (!els.frame || !els.hint) return;
  var st = S();
  var modelId = st.runningModelId;
  var port = st.runningModelPort;

  // 无运行中的模型：显示提示
  if (!modelId || !port) {
    framedModelId = null;
    els.frame.removeAttribute("src");
    els.frame.style.display = "none";
    els.hint.style.display = "flex";
    els.hint.textContent = _t("请先在首页启动模型，启动后即可在此对话测试");
    return;
  }

  // 同一模型且已加载：直接显示，不打断页面内状态
  if (framedModelId === modelId && els.frame.getAttribute("src")) {
    els.frame.style.display = "block";
    els.hint.style.display = "none";
    return;
  }

  // 音视频生成类模型（扩散引擎，如 MiniMax-H3）没有对话接口：不加载对话 UI，直接给出 API 地址
  var runningModel = (st.modelList || []).find(function(m) { return m.model_id === modelId; });
  if (runningModel && runningModel.support_video) {
    framedModelId = null;
    chatFrameUrl = "";
    els.frame.removeAttribute("src");
    els.frame.style.display = "none";
    els.hint.style.display = "flex";
    els.hint.textContent = _t("当前模型为音视频生成模型，请通过视频生成 API 调用：") +
      "http://127.0.0.1:" + port + "/v1/videos（异步任务，详见部署文档）";
    return;
  }

  els.hint.style.display = "flex";
  els.hint.textContent = _t("测试页面加载中...");
  try {
    var model = (st.modelList || []).find(function(m) { return m.model_id === modelId; });
    var vision = !!(model && model.support_images);
    var url = await invoke()("start_test_ui", { port: port, modelId: modelId, vision: vision });
    if (seq !== loadSeq) return;
    chatFrameUrl = url;
    framedModelId = modelId;
    // 附加 cache-buster：同一地址也能强制 iframe 重新加载（SvelteKit base 按路径计算，不受 query 影响）
    els.frame.src = url + (url.indexOf("?") === -1 ? "?" : "&") + "t=" + Date.now();
    els.frame.style.display = "block";
  } catch (e) {
    if (seq !== loadSeq) return;
    framedModelId = null;
    els.frame.style.display = "none";
    els.hint.style.display = "flex";
    els.hint.textContent = _t("测试页面加载失败: ") + e;
  }
}

function handleTauriEvent(type, payload) {
  if (type === "model-started" || type === "model-stopped") {
    framedModelId = null;
    chatFrameUrl = "";
    if (type === "model-stopped") {
      // 解绑代理目标：/props 与 /v1/* 返回 503，避免指向已停止的端口
      invoke()("stop_test_ui").catch(function() {});
    }
    loadChatFrame();
  }
}

export default {
  template,
  mount(root) {
    root.innerHTML = template;

    var els = chatEls();
    if (els.frame) {
      els.frame.addEventListener("load", function() {
        if (els.frame.getAttribute("src")) els.hint.style.display = "none";
      });
    }
    if (els.reload) {
      els.reload.addEventListener("click", function() {
        framedModelId = null;
        loadChatFrame();
      });
    }
    if (els.open) {
      els.open.addEventListener("click", function() {
        if (chatFrameUrl) window.openUrl(chatFrameUrl);
      });
    }

    loadChatFrame();
  },
  unmount() {},
  handleTauriEvent: handleTauriEvent
};
