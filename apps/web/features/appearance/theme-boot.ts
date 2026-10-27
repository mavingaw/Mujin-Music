/**
 * The theme cache's key and the boot script, with no imports, so the root
 * layout (a server component) can inline the script.
 */
export const THEME_CACHE_KEY = "mujin.theme.v1";
export const THEME_STYLE_ID = "mj-theme";

/**
 * Runs in <head> before the page paints (the server adds its CSP nonce).
 * Plain ES5, no imports: it reads the cache, picks the dark or light form
 * and writes the style element and <html> attributes. The TV display page
 * keeps its own fixed look. The cache key and style id are written out as
 * literals (theme-cache.test.ts keeps them equal to the constants above).
 */
export const THEME_BOOT_SCRIPT = `(function(){try{if(location.pathname.indexOf("/display")===0)return;var raw=localStorage.getItem("mujin.theme.v1");if(!raw)return;var c=JSON.parse(raw);if(!c||c.v!==1)return;var dark=window.matchMedia&&window.matchMedia("(prefers-color-scheme: dark)").matches;var e=dark?c.dark:c.light;if(!e||!e.css)return;var s=document.createElement("style");s.id="mj-theme";s.textContent=e.css;document.head.appendChild(s);var r=document.documentElement;for(var k in e.attrs){if(e.attrs[k]==null)r.removeAttribute(k);else r.setAttribute(k,e.attrs[k]);}var m=document.querySelector('meta[name="theme-color"]');if(m&&e.themeColor)m.setAttribute("content",e.themeColor);}catch(x){}})();`;
