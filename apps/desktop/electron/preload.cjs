const { contextBridge, ipcRenderer } = require("electron");

let serverMode = null;
try {
  // Synchronous so the web app knows before its first render whether this
  // window shows the server this app runs ("local") or a remote one.
  serverMode = ipcRenderer.sendSync("parson:server-mode");
} catch {}

const serverAction = (action, args) =>
  ipcRenderer.invoke("parson:servers", action, args);

contextBridge.exposeInMainWorld("__PARSON_ELECTRON__", {
  serverMode,
  invoke(command, args) {
    return ipcRenderer.invoke("parson:invoke", command, args);
  },
  windowControls: {
    close: () => ipcRenderer.invoke("parson:window-control", "close"),
    isMaximized: () =>
      ipcRenderer.invoke("parson:window-control", "is-maximized"),
    minimize: () => ipcRenderer.invoke("parson:window-control", "minimize"),
    toggleMaximize: () =>
      ipcRenderer.invoke("parson:window-control", "toggle-maximize"),
    watchMaximized: (callback) => {
      ipcRenderer.on("parson:maximized", (_event, maximized) =>
        callback(Boolean(maximized)),
      );
    },
  },
  watchPowerResume: (callback) => {
    const listener = () => callback();
    ipcRenderer.on("parson:power-resume", listener);
    return () => ipcRenderer.removeListener("parson:power-resume", listener);
  },
  watchUpdates: (callback) => {
    const listener = (_event, state) => callback(state);
    ipcRenderer.on("parson:updates", listener);
    return () => ipcRenderer.removeListener("parson:updates", listener);
  },
  watchServer: (callback) => {
    const listener = (_event, event) => callback(event);
    ipcRenderer.on("parson:server-event", listener);
    return () => ipcRenderer.removeListener("parson:server-event", listener);
  },
  servers: {
    state: () => serverAction("state"),
    discover: () => serverAction("discover"),
    connect: (target) => serverAction("connect", target),
    useLocal: () => serverAction("use-local"),
    forget: (id) => serverAction("forget", { id }),
    back: () => serverAction("back"),
    retry: () => serverAction("retry"),
    choose: () => serverAction("choose"),
    watch: (callback) => {
      const listener = (_event, state) => callback(state);
      ipcRenderer.on("parson:servers-state", listener);
      return () => ipcRenderer.removeListener("parson:servers-state", listener);
    },
  },
  startup: {
    openLogs: () => ipcRenderer.invoke("parson:startup-action", "open-logs"),
    retry: () => ipcRenderer.invoke("parson:startup-action", "retry"),
  },
});
