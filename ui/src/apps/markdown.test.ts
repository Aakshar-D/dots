// Projects' Markdown for a task's spec: everything escaped first, then a small set of markup.
import { describe, expect, it } from "vitest";

import { markdown } from "../../../crates/dots-core/apps/projects/ui/markdown.js";

describe("a spec's Markdown", () => {
  it("escapes HTML, so a spec cannot add elements or handlers of its own", () => {
    const html = markdown('<img src=x onerror="alert(1)"> & <script>bad()</script>');
    expect(html).toBe(
      "<p>&lt;img src=x onerror=&quot;alert(1)&quot;&gt; &amp; &lt;script&gt;bad()&lt;/script&gt;</p>",
    );
    const box = document.createElement("div");
    box.innerHTML = markdown("**<b onclick=x>hi</b>** [a](javascript:alert`1`) `<i>`");
    expect(box.querySelector("b, i, a, script, img")).toBeNull();
    expect(box.textContent).toBe("<b onclick=x>hi</b> a <i>");
  });

  it("writes headings, paragraphs, quotes and code", () => {
    expect(markdown("# Goal\nShip it\nsoon\n\n> wait\n> here")).toBe(
      "<h4>Goal</h4>\n<p>Ship it<br>soon</p>\n<blockquote>wait<br>here</blockquote>",
    );
    expect(markdown("```\nlet a = 1 < 2;\n**not bold**\n```")).toBe(
      "<pre><code>let a = 1 &lt; 2;\n**not bold**</code></pre>",
    );
    expect(markdown("Use `**x**` and **bold**, *it* and _it_ in snake_case_name")).toBe(
      "<p>Use <code>**x**</code> and <strong>bold</strong>, <em>it</em> and <em>it</em> in " +
        "snake_case_name</p>",
    );
  });

  it("writes lists, numbered lists and checkboxes", () => {
    expect(markdown("- one\n- [x] two\n- [ ] three\n\n1. first\n2. second")).toBe(
      '<ul><li>one</li><li class="task">☑ two</li><li class="task">☐ three</li></ul>\n' +
        "<ol><li>first</li><li>second</li></ol>",
    );
  });

  it("shows a link's text with its address as a title", () => {
    expect(markdown("See [the docs](https://example.com/a?b=1&c=2)")).toBe(
      '<p>See <span class="link" title="https://example.com/a?b=1&amp;c=2">the docs</span></p>',
    );
  });
});
