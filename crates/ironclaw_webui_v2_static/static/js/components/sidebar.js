import { Link } from "react-router";
import { html } from "../lib/html.js";
import { isDesktopRuntime } from "../lib/api.js";
import { appScopedPath } from "../lib/app-path.js";
import { SidebarFooter } from "./sidebar-footer.js";
import { SidebarNav } from "./sidebar-nav.js";
import { SidebarThreads } from "./sidebar-threads.js";
import { SidebarTraceCredits } from "./sidebar-trace-credits.js";

export function Sidebar({
  threadsState,
  theme,
  toggleTheme,
  profile,
  isAdmin,
  onSignOut,
  onClose,
  onNewChat,
  onSelectThread,
  onDeleteThread,
}) {
  return html`
    <aside
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div
        data-tauri-drag-region
        className=${`flex items-center gap-2.5 px-4 pb-4 ${isDesktopRuntime() ? "pt-10" : "pt-5"}`}
      >
        <${Link}
          to="/chat"
          onClick=${onClose}
          className="flex min-h-[44px] min-w-[44px] items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src=${appScopedPath("/assets/logo.jpg")} alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${SidebarNav}
        onNewChat=${onNewChat}
        isCreating=${threadsState.isCreating}
        isAdmin=${isAdmin}
        onNavigate=${onClose}
      />

      <${SidebarTraceCredits} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${SidebarThreads}
          threads=${threadsState.threads}
          activeThreadId=${threadsState.activeThreadId}
          isLoading=${threadsState.isLoading}
          isError=${threadsState.isError}
          onRetry=${threadsState.refetch}
          hasMore=${threadsState.hasMoreThreads}
          isLoadingMore=${threadsState.isLoadingMore}
          onLoadMore=${threadsState.loadMoreThreads}
          onSelect=${onSelectThread}
          onDelete=${onDeleteThread}
        />
      </div>

      <${SidebarFooter}
        theme=${theme}
        toggleTheme=${toggleTheme}
        profile=${profile}
        onSignOut=${onSignOut}
      />
    </aside>
  `;
}
