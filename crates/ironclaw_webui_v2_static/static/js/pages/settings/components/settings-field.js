import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Card } from "../../../design-system/card.js";

function SavedIndicator({ visible }) {
  const t = useT();
  if (!visible) return null;
  return html`
    <span className="text-[11px] font-medium text-[var(--v2-positive-text)]" role="status">
      ${t("tools.saved")}
    </span>
  `;
}

function Toggle({ checked, onChange, label }) {
  // Mobile-first 44px hit target (global touch-target law): the button fills a
  // 44px-tall row on touch widths and centers the compact 24px track; desktop
  // collapses to the dense 24px control where pointer precision is fine.
  return html`
    <button
      type="button"
      role="switch"
      aria-checked=${checked}
      aria-label=${label}
      onClick=${() => onChange(!checked)}
      className="group flex h-11 w-11 shrink-0 cursor-pointer items-center justify-center rounded-full focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)] md:h-6"
    >
      <span
        className=${[
          "relative inline-flex h-6 w-11 rounded-full border",
          checked
            ? "border-[color-mix(in_srgb,var(--v2-accent)_46%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]"
            : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)]",
        ].join(" ")}
      >
        <span
          className=${[
            "pointer-events-none inline-block h-5 w-5 rounded-full",
            checked
              ? "translate-x-5 bg-[var(--v2-accent)]"
              : "translate-x-0 bg-[var(--v2-text-faint)]",
          ].join(" ")}
        />
      </span>
    </button>
  `;
}

export function SettingsField({ field, value, onSave, isSaved }) {
  const t = useT();
  const [localValue, setLocalValue] = React.useState("");
  const label = field.labelKey ? t(field.labelKey) : field.label || "";
  const description = field.descKey ? t(field.descKey) : field.description || "";
  const fixedSingleOption =
    field.type === "select" && field.allowDefault === false && field.options?.length === 1;
  const selectValue = fixedSingleOption ? field.options[0] : localValue;
  const fixedSavedRef = React.useRef(false);

  React.useEffect(() => {
    if (field.type !== "boolean") {
      setLocalValue(value !== null && value !== undefined ? String(value) : "");
    }
  }, [value, field.type]);

  React.useEffect(() => {
    if (!fixedSingleOption) {
      fixedSavedRef.current = false;
      return;
    }
    if (value === null || value === undefined || value === "") return;
    if (value === selectValue) return;
    if (fixedSavedRef.current) return;
    fixedSavedRef.current = true;
    onSave(field.key, selectValue);
  }, [field.key, fixedSingleOption, onSave, selectValue, value]);

  const handleCommit = React.useCallback(
    (val) => {
      if (val === "") {
        onSave(field.key, null);
      } else if (field.type === "number") {
        const parsed = parseInt(val, 10);
        if (!isNaN(parsed)) onSave(field.key, parsed);
      } else if (field.type === "float") {
        const parsed = parseFloat(val);
        if (!isNaN(parsed)) onSave(field.key, parsed);
      } else {
        onSave(field.key, val);
      }
    },
    [field.key, field.type, onSave]
  );

  return html`
    <div
      className="flex items-start justify-between gap-6 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-[var(--v2-text-strong)]">${label}</div>
        ${description &&
        html`<div className="mt-1 text-xs leading-5 text-[var(--v2-text-muted)]">
          ${description}
        </div>`}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${field.type === "boolean"
          ? html`
              <${Toggle}
                checked=${value === true || value === "true"}
                onChange=${(v) => onSave(field.key, v ? "true" : "false")}
                label=${label}
              />
            `
          : field.type === "select"
            ? html`
                <select
                  value=${selectValue}
                  disabled=${fixedSingleOption}
                  onChange=${(e) => {
                    setLocalValue(e.target.value);
                    handleCommit(e.target.value);
                  }}
                  aria-label=${label}
                  className="v2-select h-11 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3 text-sm text-[var(--v2-text-strong)] outline-none focus:border-[var(--v2-accent)] disabled:opacity-100 md:h-9"
                >
                  ${!fixedSingleOption && html`<option value="">${t("tools.default")}</option>`}
                  ${field.options.map(
                    (opt) =>
                      html`<option key=${opt} value=${opt}>
                        ${field.optionLabels?.[opt] || opt}
                      </option>`
                  )}
                </select>
              `
            : html`
                <input
                  type=${field.type === "float" || field.type === "number" ? "number" : "text"}
                  value=${localValue}
                  onChange=${(e) => setLocalValue(e.target.value)}
                  onBlur=${(e) => handleCommit(e.target.value)}
                  onKeyDown=${(e) => e.key === "Enter" && handleCommit(e.target.value)}
                  step=${field.step !== undefined
                    ? String(field.step)
                    : field.type === "float"
                      ? "any"
                      : "1"}
                  min=${field.min !== undefined ? String(field.min) : undefined}
                  max=${field.max !== undefined ? String(field.max) : undefined}
                  placeholder=${t("tools.default")}
                  aria-label=${label}
                  className="h-11 w-36 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3 text-right font-mono text-sm text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)] md:h-9"
                />
              `}
        <${SavedIndicator} visible=${isSaved} />
      </div>
    </div>
  `;
}

export function SettingsGroup({ group, groupKey, fields, settings, onSave, savedKeys }) {
  const t = useT();
  const groupLabel = groupKey ? t(groupKey) : group || "";
  return html`
    <${Card} className="p-4 sm:p-6">
      <h3
        className="mb-4 text-xs font-semibold uppercase tracking-[0.08em] text-[var(--v2-accent-text)]"
      >
        ${groupLabel}
      </h3>
      <div>
        ${fields.map(
          (field) => html`
            <${SettingsField}
              key=${field.key}
              field=${field}
              value=${settings[field.key]}
              onSave=${onSave}
              isSaved=${savedKeys[field.key]}
            />
          `
        )}
      </div>
    <//>
  `;
}
