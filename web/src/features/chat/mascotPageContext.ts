const MAX_HEADING_COUNT = 8;
const MAX_CONTROL_COUNT = 18;
const MAX_VISIBLE_TEXT_CHARS = 2200;

function isVisibleElement(element: Element): element is HTMLElement {
  if (!(element instanceof HTMLElement)) {
    return false;
  }

  const style = window.getComputedStyle(element);
  if (
    style.display === "none" ||
    style.visibility === "hidden" ||
    style.opacity === "0" ||
    element.hidden ||
    element.getAttribute("aria-hidden") === "true"
  ) {
    return false;
  }

  const rect = element.getBoundingClientRect();
  return rect.width > 0 && rect.height > 0;
}

function normalizeWhitespace(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function truncateValue(value: string, maxChars: number): string {
  return value.length <= maxChars ? value : `${value.slice(0, maxChars - 1)}...`;
}

function looksSensitive(value: string): boolean {
  return /(api[\s_-]*key|token|secret|password|bearer|authorization)/i.test(value);
}

function safeControlValue(
  element: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement,
): string | null {
  if (
    element instanceof HTMLInputElement &&
    ["password", "hidden", "file"].includes(element.type)
  ) {
    return null;
  }

  const labelText = normalizeWhitespace(
    [
      element.getAttribute("aria-label"),
      element.getAttribute("placeholder"),
      element.getAttribute("name"),
      element.id,
    ]
      .filter(Boolean)
      .join(" "),
  );

  if (looksSensitive(labelText)) {
    return "[redacted]";
  }

  if (element instanceof HTMLInputElement) {
    if (["checkbox", "radio"].includes(element.type)) {
      return element.checked ? "checked" : "unchecked";
    }
    return element.value ? truncateValue(normalizeWhitespace(element.value), 90) : null;
  }

  if (element instanceof HTMLSelectElement) {
    const selected = element.selectedOptions.item(0)?.textContent ?? "";
    return selected ? truncateValue(normalizeWhitespace(selected), 90) : null;
  }

  return element.value ? truncateValue(normalizeWhitespace(element.value), 90) : null;
}

function firstText(
  values: Array<string | null | undefined>,
  fallback = "",
): string {
  for (const value of values) {
    const normalized = normalizeWhitespace(value ?? "");
    if (normalized) {
      return normalized;
    }
  }

  return fallback;
}

function collectHeadings(root: ParentNode): string[] {
  const headings = Array.from(root.querySelectorAll("h1, h2, h3"))
    .filter(isVisibleElement)
    .map((element) => truncateValue(normalizeWhitespace(element.textContent ?? ""), 120))
    .filter(Boolean);

  return Array.from(new Set(headings)).slice(0, MAX_HEADING_COUNT);
}

function collectControls(root: ParentNode): string[] {
  const controls = Array.from(
    root.querySelectorAll("button, a[href], input, textarea, select, [role='button']"),
  )
    .filter(isVisibleElement)
    .map((element) => {
      if (
        element instanceof HTMLInputElement ||
        element instanceof HTMLTextAreaElement ||
        element instanceof HTMLSelectElement
      ) {
        const label = firstText([
          element.getAttribute("aria-label"),
          element.getAttribute("placeholder"),
          element.getAttribute("name"),
          element.id,
        ]);
        if (!label) {
          return null;
        }

        const value = safeControlValue(element);
        return value ? `input: ${label} = ${value}` : `input: ${label}`;
      }

      const kind = element.tagName.toLowerCase() === "a" ? "link" : "button";
      const label = firstText([
        element.getAttribute("aria-label"),
        element.getAttribute("title"),
        element.textContent,
      ]);
      return label ? `${kind}: ${truncateValue(label, 100)}` : null;
    })
    .filter((value): value is string => Boolean(value));

  return Array.from(new Set(controls)).slice(0, MAX_CONTROL_COUNT);
}

function collectVisibleText(root: ParentNode): string {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (node) => {
      const parent = node.parentElement;
      if (!parent || !isVisibleElement(parent)) {
        return NodeFilter.FILTER_REJECT;
      }

      if (
        ["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE"].includes(parent.tagName) ||
        parent.closest(".chat-mascot-menu")
      ) {
        return NodeFilter.FILTER_REJECT;
      }

      const text = normalizeWhitespace(node.textContent ?? "");
      return text ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
    },
  });

  const chunks: string[] = [];
  let currentLength = 0;
  let currentNode = walker.nextNode();
  while (currentNode && currentLength < MAX_VISIBLE_TEXT_CHARS) {
    const text = normalizeWhitespace(currentNode.textContent ?? "");
    if (text) {
      const next = truncateValue(text, 200);
      chunks.push(next);
      currentLength += next.length + 1;
    }
    currentNode = walker.nextNode();
  }

  return truncateValue(Array.from(new Set(chunks)).join(" | "), MAX_VISIBLE_TEXT_CHARS);
}

export function captureCurrentPageContext(): string {
  const root =
    document.querySelector("main") ??
    document.querySelector("[role='main']") ??
    document.querySelector(".page") ??
    document.body;

  const headings = collectHeadings(root);
  const controls = collectControls(root);
  const visibleText = collectVisibleText(root);
  const title = normalizeWhitespace(document.title);
  const pathname = window.location.pathname + window.location.search;

  const sections = [
    `URL: ${window.location.href}`,
    title ? `Title: ${title}` : null,
    pathname ? `Route: ${pathname}` : null,
    headings.length > 0 ? `Headings:\n- ${headings.join("\n- ")}` : null,
    controls.length > 0 ? `Visible controls:\n- ${controls.join("\n- ")}` : null,
    visibleText ? `Visible text:\n${visibleText}` : null,
  ].filter(Boolean);

  return sections.join("\n\n");
}
