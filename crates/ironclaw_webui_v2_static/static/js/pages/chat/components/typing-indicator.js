import { html } from "../../../lib/html.js";
import { Avatar } from "./avatar.js";
import { useT } from "../../../lib/i18n.js";

// Shown while a run is in flight (chat.js mounts it on isProcessing). The user
// must be able to SEE that IronClaw is working — silent dots alone read as a
// hung app — so the assistant identity is announced alongside the live dots.
//
// Accessibility (from desktop): role=status + aria-live=polite announces the
// in-flight state to assistive tech, and the dots are aria-hidden decoration.
// The visible/announced label stays localized via useT so the web build never
// regresses to hardcoded English.
export function TypingIndicator() {
  const t = useT();
  const label = t("chat.identityAssistant");
  return html`
    <div
      className="flex flex-col items-start"
      role="status"
      aria-live="polite"
      aria-label=${label}
    >
      <div className="flex min-w-0 max-w-[85%] flex-col gap-2">
        <div className="flex items-center gap-2 px-1">
          <${Avatar} role="assistant" />
          <span className="text-xs font-medium text-[var(--v2-text-muted)]">
            ${label}
          </span>
        </div>
        <div
          className="w-fit rounded-[18px] border border-white/10 bg-iron-800/60 px-4 py-3"
        >
          <div className="flex gap-1" aria-hidden="true">
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
          </div>
        </div>
      </div>
    </div>
  `;
}
