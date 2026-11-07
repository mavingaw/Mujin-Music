// Gives the page a small, fixed set of desktop abilities and nothing else.
const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("desktop", {
  isDesktop: true,
  redirectUrl: "https://localhost:55555/",
  origin: "https://localhost:55555",
  request: (options) => ipcRenderer.invoke("bungie:request", options),
  signIn: (authorizeUrl) => ipcRenderer.invoke("bungie:sign-in", authorizeUrl),
});
