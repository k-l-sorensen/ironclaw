// Desktop product-voice overrides, keyed by `[lang][i18nKey] -> value`.
//
// These are the keys where IronClaw Desktop reframes shared web copy into
// its product voice (Automations -> Scheduled, MCP -> Knowledge, "LLM
// providers" -> "NEAR AI Cloud", and so on). The canonical web copy lives in
// the base locale packs (js/i18n/<lang>.js) and is the source of truth for the
// hosted console; it must never be overwritten to give desktop its voice.
//
// i18n strings cannot be runtime-gated inline at each call site, so the gating
// happens here: lib/i18n.js consults this map ONLY when `isDesktopRuntime()` is
// true, layering these values over the base pack per key. On web the map is
// never read, so web copy is unchanged.
//
// Only `en` is reframed today. Non-English locales intentionally have no
// entries: on desktop they fall through to their fully-translated base pack
// rather than a desktop-voiced-but-English string (a localization regression).
// Add a locale here only with a real translation of the desktop voice.
export const DESKTOP_OVERRIDES = {
  en: {
    "automations.description": "Recurring work IronClaw runs on a schedule. IronClaw sets these up during a conversation, so ask in chat to create one.",
    "automations.summary.pausedDetail": "Schedules currently not expected to run.",
    "automations.title": "Scheduled",
    "chat.emptyDesc": "Send a message, attach files, or ask IronClaw to draft, summarize, reconcile, or follow up.",
    "chat.emptyTitle": "Start with the work you actually need done.",
    "chat.heroDesc": "Give it the document, notes, spreadsheet, or messy ask. IronClaw keeps inputs, approvals, and outputs visible.",
    "chat.heroPlaceholder": "Hand IronClaw a document, note, or task…",
    "chat.suggestion1": "Summarize a document",
    "chat.suggestion1Desc": "Drop in a PDF, Word, or Excel file — get the key points and action items.",
    "chat.suggestion2": "Draft from notes",
    "chat.suggestion2Desc": "Turn rough notes into a clean email, memo, or update.",
    "chat.suggestion3": "Analyze a spreadsheet",
    "chat.suggestion3Desc": "Attach an XLSX or CSV — get totals, outliers, and trends.",
    "ext.channels": "Messaging",
    "ext.installed": "My apps",
    "ext.mcp": "Knowledge",
    "ext.registry": "Browse",
    "ext.registry.availableTitle": "Available apps",
    "ext.registry.emptyDesc": "IronClaw cannot reach the app catalog, or every available app is already connected.",
    "ext.registry.emptyTitle": "No apps available yet",
    "ext.registry.noMatch": "No apps match the filter.",
    "ext.registry.searchPlaceholder": "Search apps...",
    "extensions.channels": "Messaging",
    "extensions.installed": "My apps",
    "extensions.mcp": "Knowledge",
    "extensions.registry": "Browse",
    "inference.backend": "Model access",
    "inference.model": "Active model",
    "inference.provider": "AI runtime",
    "jobs.list.empty.noJobsDesc": "Background work, sandbox runs, and recovery requests will appear here once the gateway starts creating jobs.",
    "llm.groupReady": "Available",
    "llm.missingApiKey": "Sign in or use NEAR API key",
    "llm.providers": "NEAR AI Cloud",
    "llm.providersDesc": "IronClaw routes model access through NEAR AI Cloud. No third-party API keys are required for normal use.",
    "login.bearerAuth": "Access token",
    "login.bearerDesc": "Paste your IronClaw token to open the desktop app.",
    "login.console": "IronClaw",
    "login.hero": "Your chief of staff for work that needs doing.",
    "login.heroSub": "Chat, connect your workspace, and keep approvals visible while IronClaw works.",
    "login.secureSub": "Secure access to your workspace assistant.",
    "login.tagline": "IronClaw Desktop",
    "login.tokenHint": "Use the token from your IronClaw session.",
    "login.tokenLabel": "Access token",
    "login.tokenRequired": "Access token is required",
    "logs.empty": "IronClaw isn't streaming activity to this view yet. Your work, approvals, and receipts stay in Chat.",
    "nav.automations": "Scheduled",
    "nav.extensions": "Connections",
    "nav.sectionSystem": "Manage",
    "nav.sectionWork": "Assistant",
    "onboarding.moreInSettings": "Advanced model setup stays in",
    "onboarding.nearaiWaiting": "Finish signing in to NEAR AI in your browser…",
    "onboarding.providerNearaiDesc": "Hosted model access through IronClaw.",
    "onboarding.subtitle": "An agentic chief of staff for documents, connectors, approvals, and the work you need finished.",
    "onboarding.title": "IronClaw Desktop",
    "routines.description": "Search saved routines, inspect their schedule or trigger, and run or pause them from this page.",
    "settings.inference": "AI setup",
    "skills.contentHint": "Use this instead of URL when importing local or copied skill content.",
    "skills.import": "Import skill",
    "skills.importDesc": "Install from an HTTPS SKILL.md URL or paste SKILL.md content directly.",
    "skills.install": "Import",
    "skills.installFailed": "Import failed.",
    "skills.installedSuccess": "Imported skill \"{name}\"",
    "skills.installing": "Importing...",
    "skills.noInstalledDesc": "Skills extend the agent with domain-specific instructions. Import a SKILL.md bundle or place SKILL.md files in your workspace.",
    "skills.urlHint": "Use a direct HTTPS link to SKILL.md or a supported skill bundle.",
    "tool.riskNetwork": "uses network",
    "tool.runFile": "read {n} file",
    "tool.runFiles": "read {n} files",
    "tool.runOther": "used {n} tool",
    "tool.runOthers": "used {n} tools",
    "tool.runSearch": "searched {n} time",
    "tool.runSearches": "searched {n} times"
  },
};
