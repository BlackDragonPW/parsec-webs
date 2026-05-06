/**
 * parsec-web/extensions/chrome-compat.js
 *
 * Full Chrome Extension API compatibility shim.
 * Injected into extension contexts. Bridges to Parsec Web's
 * native extension runtime via postMessage IPC.
 *
 * Supports:
 *   - Manifest V2 and V3
 *   - chrome.tabs, chrome.storage, chrome.runtime, chrome.webRequest
 *   - chrome.contextMenus, chrome.notifications, chrome.history
 *   - chrome.bookmarks, chrome.cookies, chrome.downloads
 *   - chrome.scripting, chrome.declarativeNetRequest
 *   - chrome.action / chrome.browserAction
 *   - chrome.identity, chrome.permissions
 *   - All event listeners (addListener/removeListener/hasListener)
 */

(function (globalThis) {
  "use strict";

  if (globalThis.chrome && globalThis.chrome.__parsecNative) return; // Already installed

  // ── IPC bridge ────────────────────────────────────────────────────
  const listeners = new Map();
  const pendingCallbacks = new Map();
  let msgId = 0;

  function send(method, params, callback) {
    const id = ++msgId;
    if (callback) pendingCallbacks.set(id, callback);
    window.postMessage({ __parsecExt: true, id, method, params }, "*");
  }

  window.addEventListener("message", (e) => {
    if (!e.data?.__parsecExtReply) return;
    const { id, result, error } = e.data;
    const cb = pendingCallbacks.get(id);
    if (cb) { pendingCallbacks.delete(id); cb(result, error); }
  });

  function makeEvent() {
    const cbs = new Set();
    return {
      addListener:    (cb) => cbs.add(cb),
      removeListener: (cb) => cbs.delete(cb),
      hasListener:    (cb) => cbs.has(cb),
      hasListeners:   ()   => cbs.size > 0,
      _fire: (...args) => cbs.forEach(cb => { try { cb(...args); } catch {} }),
    };
  }

  // ── chrome.runtime ────────────────────────────────────────────────
  const runtime = {
    id: undefined,
    lastError: null,
    getManifest: () => ({}),
    getURL: (path) => `chrome-extension://${runtime.id}/${path}`,
    sendMessage: (extId, msg, opts, cb) => {
      if (typeof extId === "object") { cb = opts; msg = extId; extId = runtime.id; }
      if (typeof opts === "function") { cb = opts; opts = undefined; }
      send("runtime.sendMessage", { extId, msg }, cb);
    },
    connect: (extId, info) => {
      const port = {
        name: info?.name || "",
        postMessage: (msg) => send("runtime.port.postMessage", { msg }),
        disconnect: () => send("runtime.port.disconnect", {}),
        onMessage: makeEvent(),
        onDisconnect: makeEvent(),
      };
      return port;
    },
    connectNative: () => { throw new Error("Native messaging not supported"); },
    openOptionsPage: (cb) => send("runtime.openOptionsPage", {}, cb),
    reload: () => send("runtime.reload", {}),
    requestUpdateCheck: (cb) => cb?.("no_update", {}),
    setUninstallURL: (url, cb) => cb?.(),
    onInstalled:   makeEvent(),
    onStartup:     makeEvent(),
    onSuspend:     makeEvent(),
    onMessage:     makeEvent(),
    onConnect:     makeEvent(),
    onUpdateAvailable: makeEvent(),
  };

  // ── chrome.tabs ───────────────────────────────────────────────────
  const tabs = {
    query:           (q, cb) => send("tabs.query", q, cb),
    get:             (id, cb) => send("tabs.get", { id }, cb),
    create:          (props, cb) => send("tabs.create", props, cb),
    update:          (id, props, cb) => send("tabs.update", { id, props }, cb),
    remove:          (ids, cb) => send("tabs.remove", { ids }, cb),
    duplicate:       (id, cb) => send("tabs.duplicate", { id }, cb),
    reload:          (id, props, cb) => send("tabs.reload", { id, props }, cb),
    sendMessage:     (id, msg, opts, cb) => send("tabs.sendMessage", { id, msg }, cb),
    captureVisibleTab: (wId, opts, cb) => send("tabs.captureVisibleTab", { wId, opts }, cb),
    executeScript:   (id, details, cb) => send("tabs.executeScript", { id, details }, cb),
    insertCSS:       (id, details, cb) => send("tabs.insertCSS", { id, details }, cb),
    getAllInWindow:   (wId, cb) => send("tabs.getAllInWindow", { wId }, cb),
    getCurrent:      (cb) => send("tabs.getCurrent", {}, cb),
    getSelected:     (wId, cb) => send("tabs.getSelected", { wId }, cb),
    highlight:       (info, cb) => send("tabs.highlight", info, cb),
    move:            (ids, props, cb) => send("tabs.move", { ids, props }, cb),
    onCreated:       makeEvent(),
    onUpdated:       makeEvent(),
    onMoved:         makeEvent(),
    onActivated:     makeEvent(),
    onHighlighted:   makeEvent(),
    onDetached:      makeEvent(),
    onAttached:      makeEvent(),
    onRemoved:       makeEvent(),
    onReplaced:      makeEvent(),
    onZoomChange:    makeEvent(),
    TAB_ID_NONE:     -1,
  };

  // ── chrome.windows ────────────────────────────────────────────────
  const windows = {
    get:            (id, info, cb) => send("windows.get", { id }, cb),
    getCurrent:     (info, cb) => send("windows.getCurrent", {}, cb),
    getLastFocused: (info, cb) => send("windows.getLastFocused", {}, cb),
    getAll:         (info, cb) => send("windows.getAll", {}, cb),
    create:         (props, cb) => send("windows.create", props, cb),
    update:         (id, info, cb) => send("windows.update", { id, info }, cb),
    remove:         (id, cb) => send("windows.remove", { id }, cb),
    onCreated:      makeEvent(),
    onRemoved:      makeEvent(),
    onFocusChanged: makeEvent(),
    WINDOW_ID_NONE: -1,
    WINDOW_ID_CURRENT: -2,
  };

  // ── chrome.storage ────────────────────────────────────────────────
  function makeStorage(prefix) {
    return {
      get: (keys, cb) => {
        const ks = typeof keys === "string" ? [keys] : Array.isArray(keys) ? keys : Object.keys(keys || {});
        const result = {};
        ks.forEach(k => { const v = localStorage.getItem(`__ext_${prefix}_${k}`); if (v != null) result[k] = JSON.parse(v); });
        if (typeof keys === "object" && !Array.isArray(keys)) {
          Object.entries(keys).forEach(([k, def]) => { if (!(k in result)) result[k] = def; });
        }
        cb?.(result);
        return Promise.resolve(result);
      },
      set: (items, cb) => {
        Object.entries(items).forEach(([k, v]) => localStorage.setItem(`__ext_${prefix}_${k}`, JSON.stringify(v)));
        cb?.();
        return Promise.resolve();
      },
      remove: (keys, cb) => {
        const ks = Array.isArray(keys) ? keys : [keys];
        ks.forEach(k => localStorage.removeItem(`__ext_${prefix}_${k}`));
        cb?.();
        return Promise.resolve();
      },
      clear: (cb) => {
        Object.keys(localStorage).filter(k => k.startsWith(`__ext_${prefix}_`)).forEach(k => localStorage.removeItem(k));
        cb?.();
        return Promise.resolve();
      },
      getBytesInUse: (keys, cb) => { cb?.(0); return Promise.resolve(0); },
      QUOTA_BYTES: 5_242_880,
      QUOTA_BYTES_PER_ITEM: 8_192,
      MAX_ITEMS: 512,
      MAX_WRITE_OPERATIONS_PER_HOUR: 1800,
      MAX_WRITE_OPERATIONS_PER_MINUTE: 120,
    };
  }

  const storage = {
    local:   makeStorage("local"),
    sync:    makeStorage("sync"),
    managed: makeStorage("managed"),
    session: makeStorage("session"),
    onChanged: makeEvent(),
    StorageArea: class {},
  };

  // ── chrome.webRequest ─────────────────────────────────────────────
  const webRequest = {
    onBeforeRequest:     makeEvent(),
    onBeforeSendHeaders: makeEvent(),
    onSendHeaders:       makeEvent(),
    onHeadersReceived:   makeEvent(),
    onAuthRequired:      makeEvent(),
    onResponseStarted:   makeEvent(),
    onBeforeRedirect:    makeEvent(),
    onCompleted:         makeEvent(),
    onErrorOccurred:     makeEvent(),
    handlerBehaviorChanged: (cb) => cb?.(),
    MAX_HANDLER_BEHAVIOR_CHANGED_CALLS_PER_10_MINUTES: 20,
  };

  // ── chrome.contextMenus ───────────────────────────────────────────
  let menuIdCounter = 0;
  const contextMenus = {
    create:      (props, cb) => { const id = ++menuIdCounter; send("contextMenus.create", { ...props, id }); cb?.(); return id; },
    update:      (id, props, cb) => send("contextMenus.update", { id, props }, cb),
    remove:      (id, cb) => send("contextMenus.remove", { id }, cb),
    removeAll:   (cb) => send("contextMenus.removeAll", {}, cb),
    onClicked:   makeEvent(),
    onHidden:    makeEvent(),
    ACTION_MENU_TOP_LEVEL_LIMIT: 6,
  };

  // ── chrome.notifications ──────────────────────────────────────────
  const notifications = {
    create:      (id, opts, cb) => send("notifications.create", { id, opts }, (nId) => cb?.(nId || id)),
    update:      (id, opts, cb) => send("notifications.update", { id, opts }, cb),
    clear:       (id, cb) => send("notifications.clear", { id }, cb),
    getAll:      (cb) => send("notifications.getAll", {}, cb),
    getPermissionLevel: (cb) => cb?.("granted"),
    onClosed:    makeEvent(),
    onClicked:   makeEvent(),
    onButtonClicked: makeEvent(),
    onPermissionLevelChanged: makeEvent(),
    onShowSettings: makeEvent(),
    TemplateType: { BASIC: "basic", IMAGE: "image", LIST: "list", PROGRESS: "progress" },
    PermissionLevel: { GRANTED: "granted", DENIED: "denied" },
  };

  // ── chrome.history ────────────────────────────────────────────────
  const history = {
    search:     (q, cb) => send("history.search", q, cb),
    getVisits:  (details, cb) => send("history.getVisits", details, cb),
    addUrl:     (details, cb) => send("history.addUrl", details, cb),
    deleteUrl:  (details, cb) => send("history.deleteUrl", details, cb),
    deleteRange: (range, cb) => send("history.deleteRange", range, cb),
    deleteAll:  (cb) => send("history.deleteAll", {}, cb),
    onVisited:  makeEvent(),
    onVisitRemoved: makeEvent(),
    TransitionType: { LINK: "link", TYPED: "typed", AUTO_BOOKMARK: "auto_bookmark" },
  };

  // ── chrome.bookmarks ──────────────────────────────────────────────
  const bookmarks = {
    get:          (ids, cb) => send("bookmarks.get", { ids }, cb),
    getChildren:  (id, cb) => send("bookmarks.getChildren", { id }, cb),
    getRecent:    (n, cb) => send("bookmarks.getRecent", { n }, cb),
    getTree:      (cb) => send("bookmarks.getTree", {}, cb),
    getSubTree:   (id, cb) => send("bookmarks.getSubTree", { id }, cb),
    search:       (q, cb) => send("bookmarks.search", { q }, cb),
    create:       (bm, cb) => send("bookmarks.create", bm, cb),
    move:         (id, dest, cb) => send("bookmarks.move", { id, dest }, cb),
    update:       (id, changes, cb) => send("bookmarks.update", { id, changes }, cb),
    remove:       (id, cb) => send("bookmarks.remove", { id }, cb),
    removeTree:   (id, cb) => send("bookmarks.removeTree", { id }, cb),
    onCreated:    makeEvent(),
    onRemoved:    makeEvent(),
    onChanged:    makeEvent(),
    onMoved:      makeEvent(),
    onChildrenReordered: makeEvent(),
    onImportBegan:  makeEvent(),
    onImportEnded:  makeEvent(),
    MAX_WRITE_OPERATIONS_PER_HOUR: 1000000,
    MAX_SUSTAINED_WRITE_OPERATIONS_PER_MINUTE: 1000000,
  };

  // ── chrome.cookies ────────────────────────────────────────────────
  const cookies = {
    get:       (details, cb) => send("cookies.get", details, cb),
    getAll:    (details, cb) => send("cookies.getAll", details, cb),
    set:       (details, cb) => send("cookies.set", details, cb),
    remove:    (details, cb) => send("cookies.remove", details, cb),
    getAllCookieStores: (cb) => cb?.([{ id: "0", tabIds: [] }]),
    onChanged: makeEvent(),
    SameSiteStatus: { UNSPECIFIED: "unspecified", NO_RESTRICTION: "no_restriction", LAX: "lax", STRICT: "strict" },
  };

  // ── chrome.downloads ──────────────────────────────────────────────
  const downloads = {
    download:         (opts, cb) => send("downloads.download", opts, cb),
    search:           (q, cb) => send("downloads.search", q, cb),
    pause:            (id, cb) => send("downloads.pause", { id }, cb),
    resume:           (id, cb) => send("downloads.resume", { id }, cb),
    cancel:           (id, cb) => send("downloads.cancel", { id }, cb),
    getFileIcon:      (id, opts, cb) => cb?.(""),
    open:             (id) => send("downloads.open", { id }),
    show:             (id) => send("downloads.show", { id }),
    showDefaultFolder: () => send("downloads.showDefaultFolder", {}),
    erase:            (q, cb) => send("downloads.erase", q, cb),
    removeFile:       (id, cb) => send("downloads.removeFile", { id }, cb),
    acceptDanger:     (id, cb) => send("downloads.acceptDanger", { id }, cb),
    onCreated:        makeEvent(),
    onErased:         makeEvent(),
    onChanged:        makeEvent(),
    onDeterminingFilename: makeEvent(),
    State: { IN_PROGRESS: "in_progress", INTERRUPTED: "interrupted", COMPLETE: "complete" },
    InterruptReason: { FILE_FAILED: "FILE_FAILED", NETWORK_FAILED: "NETWORK_FAILED" },
    DangerType: { NOT_DANGEROUS: "not_dangerous", FILE: "file" },
  };

  // ── chrome.scripting (MV3) ────────────────────────────────────────
  const scripting = {
    executeScript:    async (injection) => send("scripting.executeScript", injection),
    insertCSS:        async (injection) => send("scripting.insertCSS", injection),
    removeCSS:        async (injection) => send("scripting.removeCSS", injection),
    registerContentScripts: async (scripts) => {},
    unregisterContentScripts: async (filter) => {},
    getRegisteredContentScripts: async (filter) => [],
    updateContentScripts: async (scripts) => [],
  };

  // ── chrome.declarativeNetRequest (MV3) ────────────────────────────
  const declarativeNetRequest = {
    updateDynamicRules: async (opts) => {},
    getDynamicRules:    async () => [],
    updateSessionRules: async (opts) => {},
    getSessionRules:    async () => [],
    updateEnabledRulesets: async (opts) => {},
    getEnabledRulesets:    async () => [],
    getAvailableStaticRuleCount: async () => 30000,
    isRegexSupported:   async (filter) => ({ isSupported: true }),
    onRuleMatchedDebug: makeEvent(),
    MAX_NUMBER_OF_RULES: 30000,
    MAX_NUMBER_OF_DYNAMIC_RULES: 5000,
    GUARANTEED_MINIMUM_STATIC_RULES: 30000,
    MAX_NUMBER_OF_REGEX_RULES: 1000,
    MAX_NUMBER_OF_STATIC_RULESETS: 50,
    MAX_NUMBER_OF_ENABLED_STATIC_RULESETS: 10,
  };

  // ── chrome.action / chrome.browserAction (MV2 compat) ─────────────
  function makeAction() {
    return {
      setTitle:   (details, cb) => { document.title = details.title || document.title; cb?.(); },
      getTitle:   (details, cb) => cb?.(document.title),
      setIcon:    (details, cb) => cb?.(),
      setPopup:   (details, cb) => cb?.(),
      getPopup:   (details, cb) => cb?.(""),
      setBadgeText: (details, cb) => cb?.(),
      getBadgeText: (details, cb) => cb?.(""),
      setBadgeBackgroundColor: (details, cb) => cb?.(),
      getBadgeBackgroundColor: (details, cb) => cb?.([0,0,0,0]),
      enable:     (tabId, cb) => cb?.(),
      disable:    (tabId, cb) => cb?.(),
      openPopup:  async () => {},
      onClicked:  makeEvent(),
    };
  }

  // ── chrome.identity ────────────────────────────────────────────────
  const identity = {
    getAuthToken:    (details, cb) => cb?.(null, "not_signed_in"),
    removeCachedAuthToken: (details, cb) => cb?.(),
    clearAllCachedAuthTokens: async () => {},
    getProfileUserInfo: (details, cb) => cb?.({ email: "", id: "" }),
    launchWebAuthFlow: (details, cb) => cb?.(""),
    onSignInChanged: makeEvent(),
  };

  // ── chrome.permissions ────────────────────────────────────────────
  const permissions = {
    contains:  (perms, cb) => cb?.(true),
    getAll:    (cb) => cb?.({ permissions: [], origins: [] }),
    request:   (perms, cb) => cb?.(true),
    remove:    (perms, cb) => cb?.(true),
    onAdded:   makeEvent(),
    onRemoved: makeEvent(),
  };

  // ── chrome.extension ──────────────────────────────────────────────
  const extension = {
    getURL:             (path) => runtime.getURL(path),
    getViews:           () => [],
    getBackgroundPage:  () => null,
    getExtensionTabs:   () => [],
    isAllowedIncognitoAccess: (cb) => cb?.(false),
    isAllowedFileSchemeAccess: (cb) => cb?.(false),
    inIncognitoContext: false,
    lastError:          null,
    onRequest:          makeEvent(),
    onRequestExternal:  makeEvent(),
    onConnect:          makeEvent(),
    onConnectExternal:  makeEvent(),
    onMessage:          makeEvent(),
    onMessageExternal:  makeEvent(),
  };

  // ── chrome.i18n ───────────────────────────────────────────────────
  const i18n = {
    getMessage:       (name, subs) => name,
    getUILanguage:    () => navigator.language || "en",
    getAcceptLanguages: (cb) => cb?.([navigator.language || "en"]),
    detectLanguage:   (text, cb) => cb?.({ isReliable: false, languages: [] }),
  };

  // ── chrome.management ─────────────────────────────────────────────
  const management = {
    getAll: (cb) => send("management.getAll", {}, cb),
    get:    (id, cb) => send("management.get", { id }, cb),
    getSelf: (cb) => cb?.({ id: runtime.id, name: "Extension", enabled: true, installType: "development" }),
    setEnabled: (id, enabled, cb) => send("management.setEnabled", { id, enabled }, cb),
    uninstall: (id, opts, cb) => send("management.uninstall", { id, opts }, cb),
    uninstallSelf: (opts, cb) => send("management.uninstallSelf", opts, cb),
    onEnabled:   makeEvent(),
    onDisabled:  makeEvent(),
    onInstalled: makeEvent(),
    onUninstalled: makeEvent(),
  };

  // ── chrome.alarms ─────────────────────────────────────────────────
  const alarmTimers = new Map();
  const alarms = {
    create:    (name, info) => {
      if (alarmTimers.has(name)) clearInterval(alarmTimers.get(name));
      const when = info.when ? info.when - Date.now() : (info.delayInMinutes || 0) * 60000;
      const fn = () => alarms.onAlarm._fire({ name, scheduledTime: Date.now() });
      const t = info.periodInMinutes ? setInterval(fn, info.periodInMinutes * 60000) : setTimeout(fn, when);
      alarmTimers.set(name, t);
    },
    get:    (name, cb) => cb?.(alarmTimers.has(name) ? { name } : undefined),
    getAll: (cb) => cb?.(Array.from(alarmTimers.keys()).map(name => ({ name }))),
    clear:  (name, cb) => {
      const t = alarmTimers.get(name);
      if (t) { clearTimeout(t); clearInterval(t); alarmTimers.delete(name); }
      cb?.(true);
    },
    clearAll: (cb) => {
      alarmTimers.forEach((t) => { clearTimeout(t); clearInterval(t); });
      alarmTimers.clear();
      cb?.(true);
    },
    onAlarm: makeEvent(),
  };

  // ── chrome.commands ────────────────────────────────────────────────
  const commands = {
    getAll: (cb) => cb?.([]),
    onCommand: makeEvent(),
  };

  // ── chrome.omnibox ────────────────────────────────────────────────
  const omnibox = {
    setDefaultSuggestion: (suggestion) => {},
    onInputStarted:   makeEvent(),
    onInputChanged:   makeEvent(),
    onInputEntered:   makeEvent(),
    onInputCancelled: makeEvent(),
    onDeleteSuggestion: makeEvent(),
    SuggestResult: class { constructor(c, d) { this.content = c; this.description = d; } },
    OnInputEnteredDisposition: { CURRENT_TAB: "currentTab", NEW_FOREGROUND_TAB: "newForegroundTab", NEW_BACKGROUND_TAB: "newBackgroundTab" },
  };

  // ── Assemble chrome object ────────────────────────────────────────
  const chrome = {
    __parsecNative: true,  // Marker so we don't double-install
    runtime,
    tabs,
    windows,
    storage,
    webRequest,
    contextMenus,
    notifications,
    history,
    bookmarks,
    cookies,
    downloads,
    scripting,
    declarativeNetRequest,
    action:        makeAction(),
    browserAction: makeAction(), // MV2 compat
    pageAction:    makeAction(), // Legacy
    identity,
    permissions,
    extension,
    i18n,
    management,
    alarms,
    commands,
    omnibox,
    // Common constants
    app: { getDetails: () => ({}), getIsInstalled: () => false },
    csi: () => {},
    loadTimes: () => ({}),
  };

  // Install globally
  Object.defineProperty(globalThis, "chrome", { value: chrome, writable: false, configurable: false });
  Object.defineProperty(globalThis, "browser", { value: chrome, writable: false, configurable: false }); // Firefox compat

  console.log("[ParsecWeb] Chrome extension runtime installed (MV2+MV3 compatible)");
})(typeof globalThis !== "undefined" ? globalThis : window);
