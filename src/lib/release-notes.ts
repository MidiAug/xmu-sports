import { marked } from "marked";
import DOMPurify from "dompurify";

const rendered = new WeakMap<Element, string>();

/** Release text is untrusted; render Markdown without active or embedded content. */
export function renderReleaseNotes(host: HTMLElement, notes: string, openLink: (url: string) => void) {
  if (rendered.get(host) === notes) return;
  const fragment = DOMPurify.sanitize(marked.parse(notes, { async: false }), {
    RETURN_DOM_FRAGMENT: true,
    ALLOWED_TAGS: ["p", "br", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "strong", "em", "del", "blockquote", "pre", "code", "a", "hr", "table", "thead", "tbody", "tr", "th", "td"],
    ALLOWED_ATTR: ["href", "title", "start", "align"],
  });
  for (const link of fragment.querySelectorAll("a")) {
    try {
      const url = new URL(link.getAttribute("href") ?? "");
      if (url.protocol !== "https:" || url.username || url.password) throw new Error("Unsupported URL");
      link.href = url.href;
    } catch { link.replaceWith(...link.childNodes); }
  }
  host.replaceChildren(fragment);
  host.onclick = event => {
    const link = (event.target as Element).closest<HTMLAnchorElement>("a[href]");
    if (!link || !host.contains(link)) return;
    event.preventDefault();
    openLink(link.href);
  };
  rendered.set(host, notes);
}
