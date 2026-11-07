// Guardian Loadouts desktop app (Electron main process).
//
// The app's pages are served from the install folder over a private app://
// scheme. All Bungie.net requests go through this process, which adds the
// Origin header your Bungie app expects, so the page never runs into browser
// CORS rules. Signing in opens Bungie's own sign-in page in a separate window
// and catches the redirect back, so no local web server is needed.

const {
  app,
  BrowserWindow,
  ipcMain,
  net,
  protocol,
  shell,
  Menu,
} = require("electron");
const path = require("node:path");
const { pathToFileURL } = require("node:url");

const APP_ROOT = path.join(__dirname, "..");
const APP_URL = "app://guardian/index.html";
const BUNGIE = "https://www.bungie.net/";
const AUTHORIZE_PREFIX = "https://www.bungie.net/en/OAuth/Authorize";
// Must match the Redirect URL and Origin Header set on bungie.net/en/Application.
const REDIRECT_URL = "https://localhost:55555/";
const ORIGIN = "https://localhost:55555";

protocol.registerSchemesAsPrivileged([
  {
    scheme: "app",
    privileges: { standard: true, secure: true, supportFetchAPI: true },
  },
]);

let mainWindow = null;

function serveAppFiles() {
  protocol.handle("app", (request) => {
    const { pathname } = new URL(request.url);
    const file = path.normalize(
      path.join(APP_ROOT, decodeURIComponent(pathname)),
    );
    const relative = path.relative(APP_ROOT, file);
    const blocked =
      relative.startsWith("..") ||
      path.isAbsolute(relative) ||
      relative.split(path.sep).some((part) => part.startsWith("."));
    if (blocked) return new Response("Not found", { status: 404 });
    return net.fetch(pathToFileURL(file).toString());
  });
}

/** Bungie.net requests from the page. Only bungie.net is reachable. */
ipcMain.handle(
  "bungie:request",
  async (_event, { url, method, headers, body }) => {
    if (typeof url !== "string" || !url.startsWith(BUNGIE)) {
      throw new Error("Only bungie.net can be reached from this app.");
    }
    const response = await net.fetch(url, {
      method: method ?? "GET",
      headers: { ...headers, Origin: ORIGIN },
      body,
    });
    return { status: response.status, body: await response.text() };
  },
);

/**
 * Opens Bungie's sign-in page and resolves with the redirect's query string
 * (code and state) once Bungie sends the browser back.
 */
ipcMain.handle("bungie:sign-in", (_event, authorizeUrl) => {
  if (
    typeof authorizeUrl !== "string" ||
    !authorizeUrl.startsWith(AUTHORIZE_PREFIX)
  ) {
    throw new Error("Unexpected sign-in address.");
  }
  return new Promise((resolve, reject) => {
    const win = new BrowserWindow({
      parent: mainWindow,
      modal: true,
      width: 520,
      height: 760,
      title: "Sign in with Bungie.net",
      autoHideMenuBar: true,
      webPreferences: {
        // Keeps the Bungie/Steam/PlayStation/Xbox sign-in cookies so you
        // don't have to type your password every time.
        partition: "persist:bungie-sign-in",
        sandbox: true,
        contextIsolation: true,
      },
    });
    let settled = false;
    const finish = (url) => {
      if (settled || !url.startsWith(REDIRECT_URL)) return false;
      settled = true;
      resolve(new URL(url).search);
      win.close();
      return true;
    };
    const intercept = (event, url) => {
      if (finish(url)) event.preventDefault();
    };
    win.webContents.on("will-redirect", intercept);
    win.webContents.on("will-navigate", intercept);
    win.webContents.on("did-start-navigation", (event) => {
      if (finish(event.url)) event.preventDefault?.();
    });
    win.webContents.setWindowOpenHandler(({ url }) => {
      win.loadURL(url);
      return { action: "deny" };
    });
    win.on("closed", () => {
      if (!settled) reject(new Error("Sign-in was cancelled."));
    });
    win.loadURL(authorizeUrl);
  });
});

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1280,
    height: 860,
    minWidth: 380,
    minHeight: 500,
    title: "Guardian Loadouts",
    backgroundColor: "#0d0f14",
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
    },
  });

  // Links like bungie.net/en/Application open in your normal browser.
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith("https://")) shell.openExternal(url);
    return { action: "deny" };
  });
  mainWindow.webContents.on("will-navigate", (event, url) => {
    if (!url.startsWith("app://guardian/")) event.preventDefault();
  });

  mainWindow.loadURL(APP_URL);
  mainWindow.on("closed", () => {
    mainWindow = null;
  });
}

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on("second-instance", () => {
    if (!mainWindow) return;
    if (mainWindow.isMinimized()) mainWindow.restore();
    mainWindow.focus();
  });

  app.whenReady().then(() => {
    Menu.setApplicationMenu(null);
    serveAppFiles();
    createWindow();
    app.on("activate", () => {
      if (BrowserWindow.getAllWindows().length === 0) createWindow();
    });
  });

  app.on("window-all-closed", () => {
    if (process.platform !== "darwin") app.quit();
  });
}
