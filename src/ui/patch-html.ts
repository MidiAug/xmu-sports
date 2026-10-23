/** Preserve unchanged nodes (focus, selections and scroll) while updating live state. */
export function patchHtml(host: Element, html: string) {
  const template = document.createElement("template");
  template.innerHTML = html;
  function patch(parent: Node, desired: Node) {
    const children = Array.from(desired.childNodes);
    children.forEach((next, index) => {
      const current = parent.childNodes[index];
      if (!current) { parent.appendChild(next.cloneNode(true)); return; }
      if (current.nodeType !== next.nodeType || current.nodeName !== next.nodeName ||
          (current instanceof Element && next instanceof Element && current.id !== next.id)) {
        parent.replaceChild(next.cloneNode(true), current); return;
      }
      if (current instanceof Element && next instanceof Element) {
        for (const attr of Array.from(current.attributes)) {
          if (!next.hasAttribute(attr.name)) current.removeAttribute(attr.name);
        }
        for (const attr of Array.from(next.attributes)) {
          if (current.getAttribute(attr.name) !== attr.value) current.setAttribute(attr.name, attr.value);
        }
        patch(current, next);
      } else if (current.nodeValue !== next.nodeValue) current.nodeValue = next.nodeValue;
    });
    while (parent.childNodes.length > children.length) parent.removeChild(parent.lastChild!);
  }
  patch(host, template.content);
}
