import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// Reconciled message-bubble test: the desktop work-product / export / attachment
// suite PLUS the mono web-view guarantees (readable typography, neutral system
// surface, localized identity header). The component is memoized
// (`export const MessageBubble = React.memo(MessageBubbleImpl)`), so the harness
// strips the wrapper and drives `MessageBubbleImpl` directly, and injects
// `isDesktopRuntime: () => true` so the desktop-gated work-product path renders.

const messageBubbleSource = readFileSync(
  new URL("./message-bubble.js", import.meta.url),
  "utf8"
);
const appCssSource = readFileSync(
  new URL("../../../../styles/app.css", import.meta.url),
  "utf8"
);

function messageBubbleSourceForTest() {
  const lines = [];
  let skippingImport = false;
  for (const line of messageBubbleSource.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(
      line
        // Drop the memo wrapper so the test can call the component directly.
        .replace(
          "export const MessageBubble = React.memo(MessageBubbleImpl);",
          ""
        )
        .replace("function MessageBubbleImpl", "function MessageBubble")
    );
  }
  return `${lines.join(
    "\n"
  )}\nglobalThis.__testExports = { MessageBubble, GeneratedWorkProductHeader, GeneratedFileArtifactStack, GeneratedFileArtifactCard, AssistantExportActions, exportThread, messageActionRowClass, messageContentForDisplay, assistantResponseLooksLikeWorkProduct, messageOuterClass, messageShellClass, messageBodyClass, attachmentEvidenceLabel, shouldCompactAttachmentStack, visibleAttachmentsForMessage, attachmentStackSummary, attachmentStackClass, imageAttachmentDataUrl, imagePreviewsForMessage, fileAttachmentsForMessage, imageThumbnailStripClass };`;
}

function findComponentByName(node, name) {
  if (!node || typeof node !== "object" || !Array.isArray(node.values)) return null;
  if (node.values.some((value) => typeof value === "function" && value.name === name)) {
    return node;
  }
  for (const value of node.values) {
    const found = findComponentByName(value, name);
    if (found) return found;
  }
  return null;
}

function componentPropsByName(node, name) {
  const props = {};
  const start = node.values.findIndex(
    (value) => typeof value === "function" && value.name === name
  );
  for (let index = start + 1; index < node.values.length; index += 1) {
    const propName = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
    if (propName) props[propName] = node.values[index];
  }
  return props;
}

function createMessageBubbleContext(overrides = {}) {
  return {
    React: {
      useCallback: (fn) => fn,
      useState: (initial) => [initial, () => {}],
      useRef: (initial) => ({ current: initial }),
      memo: (fn) => fn,
    },
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    MarkdownRenderer() {},
    ToolActivity() {},
    Avatar() {},
    AttachmentPreviewModal() {},
    ProjectFileChips() {},
    Icon() {},
    Popover() {},
    Blob,
    useT: () => (key) => key,
    // Exercise the desktop-gated work-product / export / file-chip path.
    isDesktopRuntime: () => true,
    toast: () => {},
    saveBlob: () => {},
    buildDocxBlob: (content) =>
      new Blob([content], {
        type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      }),
    buildPdfBlob: (content) => new Blob([content], { type: "application/pdf" }),
    collectMermaidExportImages: () => Promise.resolve([]),
    downloadDocx: () => {},
    downloadHtml: () => {},
    downloadJson: () => {},
    downloadMarkdown: () => {},
    downloadPdf: () => {},
    copyWorkProduct: () => Promise.resolve(),
    saveAssistantResponseToWork: () => ({ href: "/work?item=work-1&artifact=artifact-1" }),
    saveGeneratedFileArtifactToWork: () => ({ href: "/work?item=work-1&artifact=artifact-file-1" }),
    openSavedWorkProduct: () => true,
    buildThreadJsonExport: () => "{}",
    buildThreadMarkdownExport: () => "# export",
    buildGeneratedFileBlob: (artifact) =>
      new Blob([artifact.data_base64 || artifact.content || "file"], {
        type: artifact.mime_type || "application/octet-stream",
      }),
    generatedFileArtifactsForMessage: (message) =>
      message.role === "assistant" ? message.generatedFiles || message.generated_files || [] : [],
    generatedFileKindLabel: (artifact) =>
      String(artifact.filename || "")
        .split(".")
        .pop()
        ?.toUpperCase() || "FILE",
    generatedFilePreviewAttachment: (artifact) => ({
      filename: artifact.filename,
      mime_type: artifact.mime_type,
      sourceFile: new Blob([artifact.data_base64 || "file"], {
        type: artifact.mime_type || "application/octet-stream",
      }),
    }),
    navigator: { clipboard: { writeText: () => Promise.resolve() } },
    setTimeout: () => {},
    ...overrides,
  };
}

test("conversation message bubbles use readable typography", () => {
  assert.match(
    messageBubbleSource,
    /['"`]text-base\s+leading-7['"`]/,
    "chat message content should render at a readable base size"
  );
  assert.doesNotMatch(
    messageBubbleSource,
    /['"`]text-sm\s+leading-6['"`]/,
    "chat message content should not regress to the compact body size"
  );
});

test("markdown body and code blocks inherit readable message sizing", () => {
  assert.match(
    appCssSource,
    /\.markdown-body\s*\{[^}]*font-size:\s*1em;[^}]*line-height:\s*1\.7;/,
    "markdown prose should inherit the message bubble size with readable leading"
  );
  assert.match(
    appCssSource,
    /\.markdown-body\s+pre\s+code\s*\{[^}]*font-size:\s*0\.9em;\s*line-height:\s*1\.65;/,
    "fenced code should stay close to body size instead of shrinking below readability"
  );
});

test("conversation roles render a localized identity header", () => {
  const context = createMessageBubbleContext();
  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const tree = context.globalThis.__testExports.MessageBubble({
    message: { id: "u1", role: "user", content: "Hello" },
  });
  const avatar = findComponentByName(tree, "Avatar");
  assert.ok(avatar, "user/assistant turns mount an Avatar identity header");
  assert.match(flatStrings(tree), /chat\.identityUser/);
});

test("assistant export actions receive the full thread messages", () => {
  const messages = [
    { id: "u1", role: "user", content: "Please draft an agreement." },
    { id: "a1", role: "assistant", content: "Draft ready." },
  ];
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const tree = context.globalThis.__testExports.MessageBubble({
    message: messages[1],
    messages,
  });
  const exportActions = findComponentByName(tree, "AssistantExportActions");
  const props = componentPropsByName(exportActions, "AssistantExportActions");

  assert.equal(props.content, "Draft ready.");
  assert.equal(props.messages, messages);
});

test("assistant work-product actions are visible without hover and can wrap", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const actionClass = context.globalThis.__testExports.messageActionRowClass("assistant", false);

  assert.match(actionClass, /\bopacity-100\b/);
  assert.doesNotMatch(actionClass, /\bopacity-0\b/);
  assert.match(actionClass, /\bflex-wrap\b/);
  assert.match(actionClass, /\bmax-w-full\b/);
});

test("system messages render on a neutral surface, not warning copper", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const { messageBodyClass } = context.globalThis.__testExports;
  const systemClass = messageBodyClass("system", false, false);

  assert.doesNotMatch(systemClass, /copper/);
  assert.match(systemClass, /border-\[var\(--v2-panel-border\)\]/);
  assert.match(systemClass, /bg-\[var\(--v2-surface-soft\)\]/);
  assert.match(systemClass, /text-\[var\(--v2-text-muted\)\]/);
});

test("plain assistant prose is borderless while user turns keep the blue bubble", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const { messageShellClass, messageBodyClass } = context.globalThis.__testExports;
  const assistantClass = messageBodyClass("assistant", false, false);
  const userClass = messageBodyClass("user", false, false);

  assert.doesNotMatch(assistantClass, /\bbg-/);
  assert.doesNotMatch(assistantClass, /\bborder(?:-|_|\b)/);
  assert.doesNotMatch(assistantClass, /v2-gold/);
  assert.doesNotMatch(assistantClass, /\brounded-/);
  assert.match(userClass, /\bbg-signal\/10\b/);
  assert.match(userClass, /\bborder-signal\/25\b/);
  assert.match(userClass, /rounded-\[18px\]/);
  assert.match(messageShellClass(false, false), /max-w-\[min\(760px,92vw\)\]/);
  assert.match(messageShellClass(true, false), /max-w-\[min\(680px,86vw\)\]/);
});

test("plain prose and bare lists stay borderless, real document work gets the artifact panel", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const { assistantResponseLooksLikeWorkProduct } = context.globalThis.__testExports;

  // Plain prose stays borderless.
  assert.equal(assistantResponseLooksLikeWorkProduct("assistant", "Sure, I can help."), false);
  assert.equal(
    assistantResponseLooksLikeWorkProduct("assistant", "Here are a few thoughts.\nLet me know."),
    false
  );
  // A lone list must NOT be over-promoted (chat-2 regression).
  assert.equal(
    assistantResponseLooksLikeWorkProduct("assistant", "Quick picks:\n- first\n- second\n- third"),
    false
  );
  assert.equal(
    assistantResponseLooksLikeWorkProduct("assistant", "Steps:\n1. open\n2. close"),
    false
  );
  // A single pipe row (not a real markdown table) must NOT be over-promoted.
  assert.equal(
    assistantResponseLooksLikeWorkProduct("assistant", "Inline | pipe | text\nstays prose"),
    false
  );

  // A heading still promotes.
  assert.equal(
    assistantResponseLooksLikeWorkProduct("assistant", "# Services agreement\n\n## Scope"),
    true
  );
  // A list under a heading still promotes (the heading carries it).
  assert.equal(
    assistantResponseLooksLikeWorkProduct("assistant", "## Plan\n\n- step one\n- step two"),
    true
  );
  // A real header+separator markdown table still promotes.
  assert.equal(
    assistantResponseLooksLikeWorkProduct(
      "assistant",
      "Comparison:\n| Name | Cost |\n| --- | --- |\n| A | 1 |\n| B | 2 |"
    ),
    true
  );
  assert.equal(
    assistantResponseLooksLikeWorkProduct(
      "assistant",
      "| Name | Cost |\n| :--- | ---: |\n| A | 1 |"
    ),
    true
  );
});

test("assistant markdown work product renders as a first-class artifact panel", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const {
    assistantResponseLooksLikeWorkProduct,
    messageOuterClass,
    messageShellClass,
    messageBodyClass,
  } = context.globalThis.__testExports;

  assert.equal(
    assistantResponseLooksLikeWorkProduct("assistant", "# Services agreement\n\n## Scope"),
    true
  );
  assert.equal(assistantResponseLooksLikeWorkProduct("assistant", "Sure, I can help."), false);
  assert.equal(assistantResponseLooksLikeWorkProduct("user", "# User-authored instructions"), false);
  assert.match(messageOuterClass(false, true), /\bw-full\b/);
  assert.doesNotMatch(messageOuterClass(false, false), /\bw-full\b/);
  assert.match(messageShellClass(false, true), /\bw-full\b/);
  assert.match(messageShellClass(false, true), /max-w-\[min\(860px,92vw\)\]/);
  assert.match(messageBodyClass("assistant", false, true), /bg-\[var\(--v2-card-bg\)\]/);
  assert.match(messageBodyClass("assistant", false, true), /shadow-\[var\(--v2-card-shadow\)\]/);
  assert.doesNotMatch(messageBodyClass("assistant", false, false), /bg-\[var\(--v2-card-bg\)\]/);
});

test("assistant work-product panel exposes an explicit generated artifact header", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const tree = context.globalThis.__testExports.MessageBubble({
    message: {
      id: "a1",
      role: "assistant",
      content: "# Services agreement\n\n## Scope\n\n- Draft clause",
    },
    messages: [{ id: "a1", role: "assistant", content: "# Services agreement" }],
  });
  const flat = flatStrings(tree);
  const artifactHeader = findComponentByName(tree, "GeneratedWorkProductHeader");
  const headerProps = componentPropsByName(artifactHeader, "GeneratedWorkProductHeader");
  const headerTree = context.globalThis.__testExports.GeneratedWorkProductHeader(headerProps);
  const headerFlat = flatStrings(headerTree);

  assert.ok(artifactHeader, "artifact header component is mounted in work-product panel");
  assert.equal(headerProps.title, "Services agreement");
  assert.match(headerFlat, /assistant-artifact-chip/);
  assert.match(headerFlat, /Generated document/);
  assert.match(headerFlat, /Services agreement/);
  assert.match(headerFlat, /exportable as DOCX, PDF, HTML, and JSON/);
  assert.match(headerFlat, /Copy generated document/);
  const exportActions = findComponentByName(headerTree, "AssistantExportActions");
  assert.ok(exportActions, "artifact header mounts scoped export actions");
  assert.match(headerFlat, /subjectLabel="generated document"/);
  assert.match(headerFlat, /popoverSide="bottom"/);
  assert.doesNotMatch(flat, /Save assistant response to Work/);
  assert.doesNotMatch(flat, /Export assistant response/);
});

test("assistant generated file payload renders as a file artifact chip", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const tree = context.globalThis.__testExports.MessageBubble({
    message: {
      id: "a-file",
      role: "assistant",
      content: "Draft complete. Review the attached Word file.",
      generatedFiles: [
        {
          filename: "services-agreement.docx",
          mime_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
          data_base64: "UEsDBGRvY3g=",
          size_label: "11 bytes",
        },
      ],
    },
    messages: [],
  });
  const flat = flatStrings(tree);
  const stack = findComponentByName(tree, "GeneratedFileArtifactStack");
  const stackProps = componentPropsByName(stack, "GeneratedFileArtifactStack");
  const stackTree = context.globalThis.__testExports.GeneratedFileArtifactStack(stackProps);
  const stackFlat = flatStrings(stackTree);
  const cardTree = context.globalThis.__testExports.GeneratedFileArtifactCard({
    artifact: stackProps.artifacts[0],
    onPreview: stackProps.onPreview,
  });
  const cardFlat = flatStrings(cardTree);

  assert.match(flat, /assistant-work-product/);
  assert.ok(stack, "generated file stack is mounted inside the work-product panel");
  assert.match(stackFlat, /generated-file-artifacts/);
  assert.equal(stackProps.artifacts.length, 1);
  assert.match(cardFlat, /generated-file-artifact-chip/);
  assert.match(cardFlat, /Generated file/);
  assert.match(cardFlat, /DOCX/);
  assert.match(cardFlat, /services-agreement\.docx/);
  assert.match(cardFlat, /Preview services-agreement\.docx/);
  assert.match(cardFlat, /Save services-agreement\.docx to Work/);
});

test("generated file artifact card exposes preview, save, and work actions", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const tree = context.globalThis.__testExports.GeneratedFileArtifactCard({
    artifact: {
      filename: "forecast.xlsx",
      mime_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
      data_base64: "UEsDBHhs",
      size_label: "8 bytes",
    },
    onPreview: () => {},
  });
  const flat = flatStrings(tree);

  assert.match(flat, /Generated file/);
  assert.match(flat, /XLSX/);
  assert.match(flat, /forecast\.xlsx/);
  assert.match(flat, /Preview/);
  assert.match(flat, /Save to Work/);
});

test("assistant export actions collapse formats behind one export control", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const tree = context.globalThis.__testExports.AssistantExportActions({
    content: "# Draft",
    messages: [{ id: "a1", role: "assistant", content: "# Draft" }],
  });
  const popover = findComponentByName(tree, "Popover");
  const props = componentPropsByName(popover, "Popover");

  assert.ok(popover, "export menu is rendered as a popover");
  assert.equal(props.open, false);
  assert.match(flatStrings(props.trigger), /Export/);
  assert.match(flatStrings(tree), /Save to Work/);
  assert.match(flatStrings(tree), /Thread DOCX/);
  assert.match(flatStrings(tree), /Thread PDF/);
});

test("assistant export actions can be scoped to generated documents", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const tree = context.globalThis.__testExports.AssistantExportActions({
    content: "# Draft",
    messages: [{ id: "a1", role: "assistant", content: "# Draft" }],
    subjectLabel: "generated document",
  });
  const flat = flatStrings(tree);

  assert.match(flat, /Save generated document to Work/);
  assert.match(flat, /Export generated document/);
  assert.match(flat, /Download work product/);
});

test("whole-thread exports support PDF and DOCX artifacts", () => {
  const saved = [];
  const context = createMessageBubbleContext({
    document: { title: "Thread Artifact Test" },
    saveBlob: (blob, filename) => {
      saved.push({ blob, filename });
      return `/tmp/${filename}`;
    },
    buildThreadMarkdownExport: (messages, options) =>
      [`# ${options.title}`, "", ...messages.map((message) => message.content)].join("\n"),
    buildDocxBlob: (content) =>
      new Blob([`docx:${content}`], {
        type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      }),
    buildPdfBlob: (content) => new Blob([`pdf:${content}`], { type: "application/pdf" }),
  });

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const messages = [{ role: "assistant", content: "Generated agreement ready." }];

  assert.equal(
    context.globalThis.__testExports.exportThread("docx", messages),
    "/tmp/ironclaw-chat-thread.docx"
  );
  assert.equal(
    context.globalThis.__testExports.exportThread("pdf", messages),
    "/tmp/ironclaw-chat-thread.pdf"
  );
  assert.equal(saved[0].filename, "ironclaw-chat-thread.docx");
  assert.equal(
    saved[0].blob.type,
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
  );
  assert.equal(saved[1].filename, "ironclaw-chat-thread.pdf");
  assert.equal(saved[1].blob.type, "application/pdf");
});

test("attachment-only user turns still render visible message content", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const { messageContentForDisplay } = context.globalThis.__testExports;

  assert.equal(
    messageContentForDisplay({
      role: "user",
      content: "",
      attachments: [{ filename: "brief.pdf" }],
    }),
    "Sent 1 attachment"
  );
  assert.equal(
    messageContentForDisplay({
      role: "user",
      content: "Draft against this template",
      attachments: [{ filename: "brief.pdf" }],
    }),
    "Draft against this template"
  );
});

test("message attachment evidence labels say what was retained for the model", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const { attachmentEvidenceLabel } = context.globalThis.__testExports;

  assert.equal(
    attachmentEvidenceLabel({
      filename: "services.pdf",
      embedded_text: "Clause text",
      extraction_status: "extracted_text_truncated",
    }),
    "Model-read text retained, truncated"
  );
  assert.equal(
    attachmentEvidenceLabel({
      filename: "huge.bin",
      extraction_status: "content_omitted_message_budget",
    }),
    "Sent as file metadata"
  );
  assert.equal(
    attachmentEvidenceLabel({
      filename: "brief.md",
      modelReadable: true,
    }),
    "Model-readable payload retained"
  );
});

test("previewable image attachments render above the user bubble as thumbnails", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const {
    MessageBubble,
    imageAttachmentDataUrl,
    imagePreviewsForMessage,
    fileAttachmentsForMessage,
    imageThumbnailStripClass,
  } = context.globalThis.__testExports;
  const attachments = [
    {
      filename: "signature-photo.png",
      mime_type: "image/png",
      data_base64: "iVBORw0KGgo=",
      size_label: "4 KB",
    },
    {
      filename: "template.pdf",
      mime_type: "application/pdf",
      size_label: "240 KB",
    },
    {
      filename: "reload-only.jpg",
      mime_type: "image/jpeg",
      size_label: "18 KB",
    },
  ];

  assert.equal(imageAttachmentDataUrl(attachments[0]), "data:image/png;base64,iVBORw0KGgo=");
  assert.equal(
    imageAttachmentDataUrl({ mime_type: "application/pdf", url: "https://example.com/file.pdf" }),
    ""
  );
  assert.equal(
    imageAttachmentDataUrl({ mime_type: "image/jpeg", url: "https://example.com/photo.jpg" }),
    "https://example.com/photo.jpg"
  );
  assert.deepEqual(
    Array.from(
      imagePreviewsForMessage({
        images: ["data:image/png;base64,INLINE"],
        attachments,
      }).map((preview) => preview.src)
    ),
    ["data:image/png;base64,INLINE", "data:image/png;base64,iVBORw0KGgo="]
  );
  assert.deepEqual(
    Array.from(fileAttachmentsForMessage(attachments).map((attachment) => attachment.filename)),
    ["template.pdf", "reload-only.jpg"]
  );
  assert.match(imageThumbnailStripClass(true), /justify-end/);

  const tree = MessageBubble({
    message: {
      id: "u1",
      role: "user",
      content: "Draft from this image and PDF.",
      images: ["data:image/png;base64,INLINE"],
      attachments,
    },
  });
  const flat = flatStrings(tree);

  assert.match(flat, /message-image-thumbnails/);
  assert.match(flat, /data:image\/png;base64,INLINE/);
  assert.match(flat, /data:image\/png;base64,iVBORw0KGgo=/);
  assert.match(flat, /Attached image: signature-photo\.png/);
  assert.match(flat, /template\.pdf/);
  assert.match(flat, /reload-only\.jpg/);
  assert.doesNotMatch(flat, /Preview signature-photo\.png/);
});

test("large user attachment stacks compact while preserving expandable access", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const {
    MessageBubble,
    shouldCompactAttachmentStack,
    visibleAttachmentsForMessage,
    attachmentStackSummary,
    attachmentStackClass,
  } = context.globalThis.__testExports;
  const attachments = Array.from({ length: 5 }, (_, index) => ({
    filename: `file-${index + 1}.pdf`,
    mime_type: "application/pdf",
    size_label: `${index + 1} KB`,
    embedded_text: index < 4 ? `text ${index}` : "",
  }));

  assert.equal(shouldCompactAttachmentStack("user", attachments), true);
  assert.equal(shouldCompactAttachmentStack("assistant", attachments), false);
  assert.deepEqual(
    visibleAttachmentsForMessage("user", attachments, false).map((att) => att.filename),
    ["file-1.pdf", "file-2.pdf", "file-3.pdf"]
  );
  assert.equal(visibleAttachmentsForMessage("user", attachments, true).length, 5);
  assert.equal(attachmentStackSummary(attachments), "4 files have model-readable text");
  assert.match(attachmentStackClass(true), /rounded-\[14px\]/);

  const tree = MessageBubble({
    message: {
      id: "u1",
      role: "user",
      content: "Draft from these files.",
      attachments,
    },
  });
  const flat = flatStrings(tree);

  assert.match(flat, /compact-attachment-stack/);
  assert.match(flat, /files attached/);
  assert.match(flat, /\b5\b/);
  assert.match(flat, /Show\s+2\s+more files|Show 2 more attached files/);
  assert.match(flat, /file-1\.pdf/);
  assert.match(flat, /file-3\.pdf/);
  assert.doesNotMatch(flat, /file-4\.pdf/);
  assert.doesNotMatch(flat, /file-5\.pdf/);
});

test("user action row remains quiet until hover or focus", () => {
  const context = createMessageBubbleContext();

  vm.runInNewContext(messageBubbleSourceForTest(), context);
  const actionClass = context.globalThis.__testExports.messageActionRowClass("user", true);

  assert.match(actionClass, /\bopacity-0\b/);
  assert.match(actionClass, /\bgroup-hover:opacity-100\b/);
  assert.match(actionClass, /\bfocus-within:opacity-100\b/);
});

function flatStrings(node) {
  const out = [];
  const visit = (value) => {
    if (value == null) return;
    if (typeof value === "string" || typeof value === "number") {
      out.push(String(value));
      return;
    }
    if (Array.isArray(value)) {
      value.forEach(visit);
      return;
    }
    if (typeof value === "object" && Array.isArray(value.strings)) {
      out.push(value.strings.join(" "));
      value.values.forEach(visit);
    }
  };
  visit(node);
  return out.join("\n");
}
