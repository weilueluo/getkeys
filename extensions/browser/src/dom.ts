const SVG_NS = 'http://www.w3.org/2000/svg';

/** Key glyph built with DOM APIs (no innerHTML, so page Trusted Types policies never interfere). */
export function keyGlyph(size = 14): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('width', String(size));
  svg.setAttribute('height', String(size));
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('fill', 'none');
  svg.setAttribute('aria-hidden', 'true');
  const ring = document.createElementNS(SVG_NS, 'circle');
  ring.setAttribute('cx', '8');
  ring.setAttribute('cy', '12');
  ring.setAttribute('r', '4.25');
  ring.setAttribute('stroke', 'currentColor');
  ring.setAttribute('stroke-width', '2.5');
  const bit = document.createElementNS(SVG_NS, 'path');
  bit.setAttribute('d', 'M12.5 12H20M17 12v3.5');
  bit.setAttribute('stroke', 'currentColor');
  bit.setAttribute('stroke-width', '2.5');
  bit.setAttribute('stroke-linecap', 'round');
  const dot = document.createElementNS(SVG_NS, 'circle');
  dot.setAttribute('cx', '8');
  dot.setAttribute('cy', '12');
  dot.setAttribute('r', '1.4');
  dot.setAttribute('fill', '#ea580c');
  svg.append(ring, bit, dot);
  return svg;
}

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<Record<'className' | 'textContent' | 'type' | 'title', string>> = {},
  ...children: Node[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  Object.assign(node, props);
  node.append(...children);
  return node;
}
