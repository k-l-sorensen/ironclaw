import { React, html } from "../lib/html.js";
import { useT } from "../lib/i18n.js";
import { useNavigate } from "react-router";
import { Icon } from "../design-system/icons.js";
import { primaryRoutes } from "../app/routes.js";
import { readSavedWorkItems } from "../pages/chat/lib/work-product-save.js";

const COMMAND_ROUTE_LABELS = {
  chat: "Go to Chat",
  work: "Go to Work",
  extensions: "Go to Connections",
  settings: "Go to Settings",
};

export function visibleCommandRoutes({ isAdmin = false } = {}) {
  return primaryRoutes.filter(
    (route) => !route.hidden && COMMAND_ROUTE_LABELS[route.id] && (isAdmin || route.id !== "admin")
  );
}

// Cap the number of saved-work rows the palette injects so a 500-item store
// can't flood the launcher; the title filter reaches the rest by name.
const MAX_PALETTE_WORK_ITEMS = 8;

function savedWorkActions(navigate, workItems) {
  return (Array.isArray(workItems) ? workItems : [])
    .filter((item) => item && item.id)
    .slice(0, MAX_PALETTE_WORK_ITEMS)
    .map((item) => {
      const artifactId = Array.isArray(item.artifacts) ? item.artifacts[0]?.id : null;
      const params = new URLSearchParams({ item: String(item.id) });
      if (artifactId) params.set("artifact", String(artifactId));
      return {
        id: `work-${item.id}`,
        label: item.title || "Untitled work",
        icon: "file",
        group: "Saved work",
        run: () => navigate(`/work?${params.toString()}`),
      };
    });
}

export function buildCommandPaletteActions({
  navigate,
  onNewChat,
  onToggleTheme,
  isAdmin = false,
  workItems = [],
}) {
  const navigation = visibleCommandRoutes({ isAdmin }).map((route) => ({
    id: `go-${route.id}`,
    label: COMMAND_ROUTE_LABELS[route.id],
    icon:
      {
        chat: "chat",
        work: "file",
        extensions: "plug",
        settings: "settings",
      }[route.id] || "bolt",
    group: "Navigate",
    run: () => navigate(route.path),
  }));

  // Keep same-group rows contiguous so each group prints a single header.
  return [
    {
      id: "new-chat",
      label: "New chat",
      icon: "plus",
      group: "Actions",
      run: () => onNewChat?.(),
    },
    {
      id: "toggle-theme",
      label: "Toggle theme",
      icon: "moon",
      group: "Actions",
      run: () => onToggleTheme?.(),
    },
    ...navigation,
    // Work product must be findable later (design law): saved dossiers are
    // reachable by title from the Bridge, not only via "Go to Work".
    ...savedWorkActions(navigate, workItems),
  ];
}

/* ⌘K / Ctrl+K launcher: jump to a thread, start a new chat, navigate to a
   section, or toggle the theme. Pure frontend: drives existing routes and
   thread state without executing work. */
export function CommandPalette({
  open,
  onClose,
  threadsState,
  onNewChat,
  onToggleTheme,
  isAdmin = false,
}) {
  const navigate = useNavigate();
  const t = useT();
  const [query, setQuery] = React.useState("");
  const [active, setActive] = React.useState(0);
  // Saved work lives in localStorage; re-read each time the palette opens so a
  // dossier saved this session is immediately findable by title.
  const [workItems, setWorkItems] = React.useState([]);
  const inputRef = React.useRef(null);
  const dialogRef = React.useRef(null);
  const activeRowRef = React.useRef(null);
  const restoreFocusRef = React.useRef(null);

  const commands = React.useMemo(() => {
    const actions = buildCommandPaletteActions({
      navigate,
      onNewChat,
      onToggleTheme,
      isAdmin,
      workItems,
    });
    const threads = (threadsState?.threads || []).map((thread) => ({
      id: `thread-${thread.id}`,
      label: thread.title || `Thread ${thread.id.slice(0, 8)}`,
      icon: "chat",
      group: "Threads",
      run: () => navigate(`/chat/${thread.id}`),
    }));
    return [...actions, ...threads];
  }, [threadsState, navigate, onNewChat, onToggleTheme, isAdmin, workItems]);

  const filtered = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return commands;
    return commands.filter((c) => c.label.toLowerCase().includes(q));
  }, [commands, query]);

  React.useEffect(() => {
    if (!open) return;
    // Remember what had focus so we can hand it back when the palette closes,
    // and keep keyboard users from being stranded on <body>.
    restoreFocusRef.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setQuery("");
    setActive(0);
    setWorkItems(readSavedWorkItems());
    const id = window.requestAnimationFrame(() => inputRef.current?.focus());
    return () => {
      window.cancelAnimationFrame(id);
      restoreFocusRef.current?.focus?.();
      restoreFocusRef.current = null;
    };
  }, [open]);

  React.useEffect(() => {
    setActive((i) => Math.min(i, Math.max(0, filtered.length - 1)));
  }, [filtered.length]);

  // Keep the highlighted row visible as arrow keys walk a long list.
  React.useEffect(() => {
    activeRowRef.current?.scrollIntoView({ block: "nearest" });
  }, [active, filtered.length]);

  const exec = React.useCallback(
    (command) => {
      if (!command) return;
      onClose();
      command.run();
    },
    [onClose]
  );

  const focusables = React.useCallback(() => {
    const root = dialogRef.current;
    if (!root) return [];
    return Array.from(
      root.querySelectorAll('input, button, [href], [tabindex]:not([tabindex="-1"])')
    ).filter((el) => !el.hasAttribute("disabled") && el.offsetParent !== null);
  }, []);

  const onKeyDown = React.useCallback(
    (event) => {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setActive((i) => Math.min(i + 1, filtered.length - 1));
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setActive((i) => Math.max(i - 1, 0));
      } else if (event.key === "Enter") {
        event.preventDefault();
        exec(filtered[active]);
      } else if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      } else if (event.key === "Tab") {
        // Trap focus inside the dialog so Tab never falls behind the modal.
        const items = focusables();
        if (items.length === 0) return;
        const first = items[0];
        const last = items[items.length - 1];
        const activeEl = document.activeElement;
        if (event.shiftKey && activeEl === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && activeEl === last) {
          event.preventDefault();
          first.focus();
        }
      }
    },
    [filtered, active, exec, onClose, focusables]
  );

  if (!open) return null;

  let lastGroup = null;

  return html`
    <div
      ref=${dialogRef}
      onKeyDown=${onKeyDown}
      className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]"
      role="dialog"
      aria-modal="true"
      aria-label="Command palette"
    >
      <button
        type="button"
        aria-label="Close"
        onClick=${onClose}
        className="absolute inset-0 bg-black/50"
      ></button>
      <div
        className="relative w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_30px_60px_-20px_rgba(0,0,0,0.8)]"
      >
        <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3">
          <${Icon} name="search" className="h-4 w-4 text-[var(--v2-text-faint)]" />
          <input
            ref=${inputRef}
            value=${query}
            onInput=${(e) => setQuery(e.currentTarget.value)}
            placeholder=${t("command.placeholder")}
            className="h-12 w-full border-0 bg-transparent text-sm text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)]"
          />
          <kbd
            className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]"
            >esc</kbd
          >
        </div>
        <ul className="max-h-[50vh] overflow-y-auto p-1.5">
          ${filtered.length === 0 &&
          html`<li
            className="flex flex-col items-center gap-1 px-3 py-7 text-center"
            data-testid="command-palette-empty"
          >
            <span className="text-sm font-medium text-[var(--v2-text)]">${t("command.noMatches")}</span>
            <span className="text-xs text-[var(--v2-text-faint)]">
              ${t("command.noMatchesHint")}
            </span>
          </li>`}
          ${filtered.map((command, index) => {
            const showGroup = command.group !== lastGroup;
            lastGroup = command.group;
            const isActive = index === active;
            return html`
              ${showGroup &&
              html`<li
                key=${`g-${command.group}`}
                className="px-2 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-[var(--v2-text-faint)]"
              >
                ${command.group}
              </li>`}
              <li key=${command.id} ref=${isActive ? activeRowRef : null}>
                <button
                  type="button"
                  onMouseEnter=${() => setActive(index)}
                  onClick=${() => exec(command)}
                  className=${[
                    "flex min-h-[44px] w-full items-center gap-2.5 rounded-[9px] px-2.5 py-2 text-left text-sm",
                    isActive
                      ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                      : "text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]",
                  ].join(" ")}
                >
                  <${Icon} name=${command.icon} className="h-4 w-4 shrink-0" />
                  <span className="min-w-0 truncate">${command.label}</span>
                </button>
              </li>
            `;
          })}
        </ul>
      </div>
    </div>
  `;
}
