import { QueryClientProvider } from "@tanstack/react-query";
import { createRoot } from "react-dom/client";
import { App } from "./app/app.js";
import { html } from "./lib/html.js";
import { queryClient } from "./lib/query-client.js";
import { I18nProvider } from "./lib/i18n.js";
import { bootstrapDesktopSession, isDesktopRuntime, storeToken } from "./lib/api.js";
import { maybeRunPackagedWebviewSmoke } from "./lib/packaged-smoke.js";
import { installZoomControls } from "./lib/zoom.js";
// Only the English fallback is bundled eagerly; every other locale is
// lazy-loaded on demand by I18nProvider (see lib/i18n.js `loaders`).
import "./i18n/en.js";

function renderApp() {
  createRoot(document.getElementById("v2-root")).render(html`
    <${I18nProvider}>
      <${QueryClientProvider} client=${queryClient}>
        <${App} />
      <//>
    <//>
  `);
}

async function boot() {
  // Desktop-only boot steps. Each is also internally gated (no-ops off
  // Tauri), but the `isDesktopRuntime()` guard keeps the whole packaged-shell
  // path — including the async session bootstrap round-trip — out of the web
  // boot, which renders synchronously below.
  if (isDesktopRuntime()) {
    maybeRunPackagedWebviewSmoke();
    installZoomControls();

    try {
      const desktopSession = await bootstrapDesktopSession();
      if (desktopSession?.token) {
        storeToken(desktopSession.token);
      }
    } catch (err) {
      console.warn("[ironclaw] desktop bootstrap failed; falling back to browser auth", err);
    }
  }

  renderApp();

  // Re-run after the first paint so the packaged-webview smoke harness can
  // exercise the mounted DOM. No-op on web (and after its first run).
  if (isDesktopRuntime()) {
    setTimeout(() => {
      maybeRunPackagedWebviewSmoke();
    }, 0);
  }
}

boot();
