// A task's spec in Markdown, as HTML for the side pane. Every character of the text is escaped
// first, so the only markup in the result is what this file writes: headings, paragraphs, lists
// with checkboxes, quotes, code, bold and italic. A link shows its text, with the address as a
// title: the page cannot open it (its frame opens no popups and navigates nowhere).

const ESCAPES = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

export const escape = (text) => text.replace(/[&<>"']/g, (c) => ESCAPES[c]);

/// One line's inline markup, on escaped text.
function inline(text) {
  const codes = [];
  // Code is taken out first, so nothing inside it is read as markup.
  let out = text.replace(/`([^`]+)`/g, (_, code) => {
    codes.push(code);
    return `\u0000${codes.length - 1}\u0000`;
  });
  out = out
    .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, '<span class="link" title="$2">$1</span>')
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/__([^_]+)__/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>")
    .replace(/(^|[^\w_])_([^_\s][^_]*)_(?!\w)/g, "$1<em>$2</em>");
  return out.replace(/\u0000(\d+)\u0000/g, (_, i) => `<code>${codes[Number(i)]}</code>`);
}

const LIST_ITEM = /^\s*([-*+]|\d+[.)])\s+(.*)$/;
const CHECKBOX = /^\[([ xX])\]\s+(.*)$/;

/// `text` (Markdown) as HTML.
export function markdown(text) {
  // NUL marks the code taken out of a line; the text cannot hold one.
  const lines = escape(text.replace(/\u0000/g, "").replace(/\r\n?/g, "\n")).split("\n");
  const out = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") {
      i += 1;
      continue;
    }
    if (line.startsWith("```")) {
      const code = [];
      i += 1;
      while (i < lines.length && !lines[i].startsWith("```")) code.push(lines[i++]);
      i += 1;
      out.push(`<pre><code>${code.join("\n")}</code></pre>`);
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      const level = Math.min(heading[1].length + 3, 6);
      out.push(`<h${level}>${inline(heading[2])}</h${level}>`);
      i += 1;
      continue;
    }
    if (line.startsWith("&gt;")) {
      const quote = [];
      while (i < lines.length && lines[i].startsWith("&gt;")) {
        quote.push(lines[i++].replace(/^&gt;\s?/, ""));
      }
      out.push(`<blockquote>${quote.map(inline).join("<br>")}</blockquote>`);
      continue;
    }
    const first = LIST_ITEM.exec(line);
    if (first) {
      const ordered = /\d/.test(first[1]);
      const items = [];
      while (i < lines.length && LIST_ITEM.test(lines[i])) {
        const item = LIST_ITEM.exec(lines[i++])[2];
        const box = CHECKBOX.exec(item);
        items.push(
          box
            ? `<li class="task">${box[1] === " " ? "☐" : "☑"} ${inline(box[2])}</li>`
            : `<li>${inline(item)}</li>`,
        );
      }
      const tag = ordered ? "ol" : "ul";
      out.push(`<${tag}>${items.join("")}</${tag}>`);
      continue;
    }
    const para = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].startsWith("```") &&
      !/^#{1,6}\s/.test(lines[i]) &&
      !lines[i].startsWith("&gt;") &&
      !LIST_ITEM.test(lines[i])
    ) {
      para.push(inline(lines[i++]));
    }
    out.push(`<p>${para.join("<br>")}</p>`);
  }
  return out.join("\n");
}
