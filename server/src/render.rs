#[path = "render/chunks.rs"]
mod chunks;
#[path = "render/diagrams.rs"]
mod diagrams;
#[path = "render/html_utils.rs"]
mod html_utils;
#[path = "render/markdown.rs"]
mod markdown;
#[path = "render/models.rs"]
mod models;

use chunks::{parse_chunks, render_chunk};

#[cfg(test)]
use chunks::{Chunk, parse_fence_start};
#[cfg(test)]
use diagrams::{diagram_label, render_diagram};
#[cfg(test)]
use html_utils::escape_html;

const EINK_CSS: &str = r#"
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
    font-family: Georgia, 'Times New Roman', serif;
    font-size: 28px;
    line-height: 1.6;
    color: #000;
    background: #fff;
    width: 2800px;
    min-height: 4000px;
    padding: 0;
    margin: 0;
}
#content {
    max-width: 1800px;
    margin: 24px 0 1500px 80px;
    padding: 0 32px;
}
#toc {
    border: 2px solid #111;
    background: #fafafa;
    padding: 16px 12px;
    margin: 0 0 24px;
    font-size: 15px;
    line-height: 1.5;
}
#toc.collapsed { display: none; }
#toc-toggle {
    border: 2px solid #111;
    background: #fff;
    color: #111;
    padding: 8px 16px;
    font-size: 16px;
    font-weight: bold;
    cursor: pointer;
    margin: 0 0 16px;
}
#toc h3 {
    font-size: 16px;
    margin: 0 0 8px;
    padding-bottom: 6px;
    border-bottom: 1px solid #ccc;
}
#toc a {
    display: block;
    padding: 4px 0 4px 0;
    color: #111;
    text-decoration: none;
    border-bottom: 1px solid #eee;
}
#toc a.toc-h1 { padding-left: 0; font-weight: 700; font-size: 16px; }
#toc a.toc-h2 { padding-left: 12px; font-weight: 600; }
#toc a.toc-h3 { padding-left: 28px; font-weight: normal; color: #444; }
#toc a:active { background: #e0e0e0; }
h1 { font-size: 42px; margin: 36px 0 16px; border-bottom: 2px solid #000; padding-bottom: 8px; }
h2 { font-size: 34px; margin: 28px 0 12px; }
h3 { font-size: 28px; font-weight: bold; margin: 20px 0 8px; }
p { margin: 12px 0; }
code {
    font-family: 'Courier New', monospace;
    font-size: 24px;
    background: #f4f4f4;
    padding: 3px 7px;
    border: 2px solid #999;
    border-radius: 3px;
}
pre {
    background: #f0f0f0;
    border: 1px solid #ccc;
    padding: 12px;
    margin: 16px 0;
    overflow-x: auto;
}
pre code { border: none; padding: 0; background: none; }
blockquote {
    border-left: 3px solid #333;
    padding-left: 16px;
    margin: 16px 0;
    color: #333;
}
ul, ol { margin: 12px 0; padding-left: 24px; }
li { margin: 4px 0; }
table { border-collapse: collapse; width: 100%; margin: 16px 0; }
th, td { border: 1px solid #333; padding: 8px 12px; text-align: left; }
th { font-weight: bold; background: #f0f0f0; }
img { max-width: 100%; height: auto; }
a { color: #000; text-decoration: underline; }
hr { border: none; border-top: 1px solid #333; margin: 24px 0; }
.diagram-block {
    margin: 28px 0;
    border: 1.5px solid #ccc;
    border-radius: 10px;
    background: #fff;
    box-shadow: 0 2px 8px rgba(0,0,0,0.09);
    overflow: hidden;
}
.diagram-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 16px;
    padding: 11px 16px;
    border-bottom: 1px solid #e0e0e0;
    background: #f8f8f8;
    font-size: 14px;
    font-weight: 700;
    line-height: 1.4;
    text-transform: uppercase;
    letter-spacing: 0.12em;
    color: #555;
}
.diagram-body {
    min-height: 200px;
    padding: 16px;
}
.diagram-stack {
    display: flex;
    flex-direction: column;
    gap: 12px;
}
.diagram-toolbar,
.diagram-legend,
.diagram-details {
    border: 1px solid #e0e0e0;
    background: #fafafa;
    border-radius: 6px;
    padding: 12px;
}
.diagram-toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
}
.diagram-toolbar button {
    border: 1.5px solid #999;
    border-radius: 6px;
    background: #fff;
    color: #333;
    padding: 6px 12px;
    font-size: 14px;
    cursor: pointer;
}
.diagram-toolbar-label {
    font-size: 15px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
}
.diagram-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 12px;
    align-items: center;
}
.diagram-legend-item {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
}
.diagram-legend-swatch {
    width: 32px;
    height: 16px;
    border: 1.5px solid #888;
    border-radius: 4px;
    background: #fff;
}
.diagram-details-title {
    font-size: 18px;
    font-weight: 700;
    margin-bottom: 8px;
}
.diagram-details-grid {
    display: grid;
    grid-template-columns: 110px 1fr;
    gap: 6px 10px;
    font-size: 15px;
}
.diagram-details-grid dt {
    font-weight: 700;
}
.diagram-details-grid dd {
    margin: 0;
    word-break: break-word;
}
.diagram-source,
.diagram-error {
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 18px;
    line-height: 1.5;
}
.diagram-error {
    border: 2px solid #111;
    background: #f6f6f6;
}
.diagram-placeholder {
    font-size: 20px;
    color: #333;
}
.mindmap-shell {
    display: flex;
    gap: 16px;
    align-items: stretch;
}
.mindmap-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 12px;
}
.mindmap-index {
    width: 280px;
    max-width: 280px;
    border-right: 1px solid #e0e0e0;
    padding-right: 12px;
}
.mindmap-index[hidden] {
    display: none;
}
.mindmap-index-item {
    display: block;
    width: 100%;
    text-align: left;
    background: transparent;
    border: 1.5px solid transparent;
    border-radius: 8px;
    padding: 7px 10px;
    margin: 3px 0;
    color: #222;
    font-size: 15px;
    cursor: pointer;
}
.mindmap-index-item.is-active {
    border-color: #666;
    background: #f0f0f0;
    font-weight: 700;
}
.mindmap-viewport {
    flex: 1;
    min-height: 520px;
    overflow: auto;
    border: 1px solid #ddd;
    border-radius: 6px;
    background: #fdfdfd;
}
.mindmap-canvas {
    display: block;
}
.mindmap-node {
    cursor: pointer;
}
.mindmap-node rect {
    fill: #fff;
    stroke: #111;
    stroke-width: 2;
}
.mindmap-node text, .mindmap-node-text {
    fill: #111;
    font-family: Georgia, 'Times New Roman', serif;
    font-size: 18px;
    font-weight: 600;
    text-anchor: middle;
}
.mindmap-node-badge rect,
.graph-node-badge rect {
    fill: #555;
    stroke: #555;
    stroke-width: 1;
}
.mindmap-node-badge text,
.graph-node-badge text {
    fill: #fff;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-anchor: middle;
    text-transform: uppercase;
}
.graph-node-kind {
    fill: #888;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-anchor: middle;
    text-transform: uppercase;
}
.mindmap-node.is-active rect {
    stroke-width: 3;
}
.mindmap-node-toggle {
    cursor: pointer;
}
.mindmap-node-toggle rect {
    fill: #555;
    stroke: #555;
    stroke-width: 1;
}
.mindmap-node-toggle text {
    fill: #fff;
    font-size: 16px;
    font-weight: bold;
    text-anchor: middle;
}
.mindmap-edge {
    fill: none;
    stroke: #888;
    stroke-width: 2;
}
.graph-viewport {
    min-height: 540px;
    overflow: auto;
    border: 1px solid #ddd;
    border-radius: 6px;
    background: #fdfdfd;
}
.graph-canvas {
    display: block;
}
.graph-shell {
    display: flex;
    flex-direction: column;
    gap: 12px;
}
.graph-node rect {
    fill: #fff;
    stroke: #111;
    stroke-width: 2;
    rx: 12;
    ry: 12;
}
.graph-node text {
    fill: #111;
    font-family: Georgia, 'Times New Roman', serif;
    font-size: 18px;
    font-weight: 600;
    text-anchor: middle;
}
.graph-node.is-active rect {
    stroke-width: 3;
}
.graph-edge {
    fill: none;
    stroke: #666;
    stroke-width: 2;
}
.graph-edge-label {
    font-size: 13px;
    fill: #555;
    text-anchor: middle;
}
.eink-muted {
    color: #444;
}
.diagram-unsupported {
    font-size: 18px;
    color: #444;
}
.diff-block {
    font-family: monospace;
    font-size: 22px;
    line-height: 1.5;
    border: 1px solid #ccc;
    border-radius: 4px;
    overflow-x: auto;
    margin: 16px 0;
}
.diff-add {
    border-left: 6px solid #2a2;
    padding-left: 8px;
    background: #f6fff6;
}
.diff-del {
    border-left: 6px solid #c33;
    padding-left: 8px;
    background: #fff6f6;
}
.diff-hunk {
    color: #555;
    font-style: italic;
    padding-left: 8px;
    border-left: 6px solid #aaa;
}
.diff-ctx {
    padding-left: 14px;
}
@media (max-width: 1200px) {
    .mindmap-shell {
        flex-direction: column;
    }
    .mindmap-index {
        width: auto;
        max-width: none;
        border-right: none;
        border-bottom: 1px solid #bbb;
        padding-right: 0;
        padding-bottom: 12px;
    }
    .diagram-details-grid {
        grid-template-columns: 1fr;
    }
}
.eink-anchored {
    border-left: 4px dashed #4a6fa5;
    background: #f0f4f8;
    padding-left: 12px;
    transition: background 0.2s, border-color 0.2s;
}
.eink-link-badge {
    position: absolute;
    top: -11px;
    right: -11px;
    min-width: 24px;
    height: 24px;
    border-radius: 12px;
    border: 2px solid #fff;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    font-weight: 700;
    font-family: 'Courier New', monospace;
    color: #fff;
    z-index: 100;
    padding: 0 5px;
    pointer-events: none;
    box-shadow: 0 1px 4px rgba(0,0,0,0.25);
}
@keyframes eink-link-pulse {
    0%, 100% { outline-width: 4px; }
    50% { outline-width: 8px; }
}
.eink-link-pulse {
    animation: eink-link-pulse 0.85s ease-in-out infinite;
}
.eink-link-popup {
    position: absolute;
    z-index: 9999;
    background: #fff;
    border: 2px solid #111;
    padding: 10px 14px;
    font-family: Georgia, 'Times New Roman', serif;
    font-size: 18px;
    line-height: 1.5;
    max-width: 500px;
    box-shadow: 0 4px 16px rgba(0,0,0,0.2);
    pointer-events: none;
}
"#;

const TOC_JS: &str = r##"
(function() {
  var content = document.getElementById('content');
  var toc = document.getElementById('toc');
  var toggle = document.getElementById('toc-toggle');
  var headings = content.querySelectorAll('h1, h2, h3');
  if (headings.length < 1) { toggle.style.display = 'none'; return; }
  var html = '<h3>Contents</h3>';
  headings.forEach(function(h, i) {
    var id = 'section-' + i;
    h.id = id;
    var level = h.tagName.toLowerCase();
    html += '<a href="#' + id + '" class="toc-' + level + '">' + h.textContent + '</a>';
  });
  toc.innerHTML = html;
  var open = false;
  toggle.addEventListener('click', function() {
    open = !open;
    toc.classList.toggle('collapsed', !open);
    toggle.textContent = open ? '\u00d7 Close' : '\u2630 Contents';
  });
  toc.addEventListener('click', function(e) {
    if (e.target.tagName === 'A') {
      open = false;
      toc.classList.add('collapsed');
      toggle.textContent = '\u2630 Contents';
    }
  });
  content.scrollIntoView({behavior:'instant',block:'start'});
})();
"##;

const BOOTSTRAP_JS: &str = r#"
(function() {
  let mermaidReady;
  let mermaidCounter = 0;

  function escapeHtml(text) {
    return String(text)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/\"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function parsePayload(block) {
    const script = block.querySelector('.diagram-payload');
    if (!script) return null;
    try {
      return JSON.parse(script.textContent || '{}');
    } catch (error) {
      renderError(block, 'Invalid embedded diagram payload', String(error));
      return null;
    }
  }

  function splitLabel(label) {
    return String(label || '').split('\n');
  }
  function nodeSize(label) {
    const lines = splitLabel(label);
    const maxLen = Math.max(...lines.map((l) => l.length));
    const width = Math.max(160, 28 + maxLen * 12);
    const height = 44 + lines.length * 22;
    return { lines, width, height };
  }
  function renderLabelText(svg, lines, x, y, lineHeight) {
    const text = document.createElementNS('http://www.w3.org/2000/svg', 'text');
    lines.forEach((line, i) => {
      const tspan = document.createElementNS('http://www.w3.org/2000/svg', 'tspan');
      tspan.setAttribute('x', String(x));
      tspan.setAttribute('dy', i === 0 ? '0' : String(lineHeight));
      tspan.textContent = line;
      text.appendChild(tspan);
    });
    text.setAttribute('x', String(x));
    text.setAttribute('y', String(y));
    return text;
  }

  function renderError(block, message, details) {
    const body = block.querySelector('.diagram-body');
    if (!body) return;
    body.innerHTML = '<pre class="diagram-error">' + escapeHtml(message + (details ? '\n\n' + details : '')) + '</pre>';
  }

  function renderSource(body, source) {
    body.innerHTML = '<pre class="diagram-source">' + escapeHtml(source) + '</pre>';
  }

  function loadMermaid() {
    if (!mermaidReady) {
      mermaidReady = new Promise((resolve, reject) => {
        if (!globalThis.mermaid) {
          reject(new Error('Mermaid asset did not load'));
          return;
        }
        globalThis.mermaid.initialize({
          startOnLoad: false,
          securityLevel: 'loose',
          theme: 'base',
          flowchart: { useMaxWidth: false },
          themeVariables: {
            primaryColor: '#ffffff',
            primaryBorderColor: '#111111',
            primaryTextColor: '#111111',
            lineColor: '#111111',
            secondaryColor: '#f5f5f5',
            tertiaryColor: '#ffffff',
            background: '#ffffff'
          }
        });
        resolve(globalThis.mermaid);
      });
    }
    return mermaidReady;
  }

  function createElk() {
    if (!globalThis.ELK) {
      throw new Error('ELK asset did not load');
    }
    return new globalThis.ELK();
  }

  async function renderMermaid(block, payload) {
    const body = block.querySelector('.diagram-body');
    if (!body) return;
    try {
      const mermaid = await loadMermaid();
      const id = 'mermaid-diagram-' + (++mermaidCounter);
      const rendered = await mermaid.render(id, payload.source || '');
      body.innerHTML = rendered.svg;
      if (rendered.bindFunctions) rendered.bindFunctions(body);
    } catch (error) {
      renderSource(body, payload.source || '');
    }
  }

  function buildMindmapTree(data) {
    return {
      id: '__root__',
      label: data.root || 'Mind Map',
      color: 'gray',
      children: Array.isArray(data.nodes) ? data.nodes : []
    };
  }

  function colorForNode(node) {
    const palette = {
      blue: '#c8dcf8',
      green: '#b8eecb',
      red: '#f9c4c4',
      amber: '#fde68a',
      purple: '#d8d0f8',
      slate: '#cdd1d8',
      gray: '#e4e6ea'
    };
    return palette[(node.color || '').toLowerCase()] || '#f5f5f5';
  }

  function darkenColor(hex, factor) {
    var r = parseInt(hex.slice(1,3), 16);
    var g = parseInt(hex.slice(3,5), 16);
    var b = parseInt(hex.slice(5,7), 16);
    r = Math.round(r * factor); g = Math.round(g * factor); b = Math.round(b * factor);
    return '#' + ((1<<24)|(r<<16)|(g<<8)|b).toString(16).slice(1);
  }

  function edgePattern(kind) {
    const patterns = {
      invokes: '6 4',
      submits: '3 4',
      polls: '14 4',
      depends: '10 4',
      reads: '2 4',
      writes: '12 4 4 4'
    };
    return patterns[(kind || '').toLowerCase()] || '';
  }

  function normalizeKind(value) {
    return (value || '').trim();
  }

  function badgeText(value) {
    const kind = normalizeKind(value);
    if (!kind) return '';
    return kind.length > 10 ? kind.slice(0, 10).toUpperCase() : kind.toUpperCase();
  }

  function metadataEntries(node) {
    return [
      ['id', node.id || null],
      ['kind', node.kind || null],
      ['color', node.color || null],
      ['description', node.description || null],
      ['file', node.file || null],
      ['symbol', node.symbol || null],
      ['line', node.line != null ? String(node.line) : null],
      ['href', node.href || null],
      ['notes', node.notes || null]
    ].filter(([, value]) => value);
  }

  function buildDetailsPanel(title, entries) {
    const panel = document.createElement('section');
    panel.className = 'diagram-details';
    const heading = document.createElement('div');
    heading.className = 'diagram-details-title';
    heading.textContent = title;
    panel.appendChild(heading);
    if (!entries.length) {
      const empty = document.createElement('div');
      empty.className = 'eink-muted';
      empty.textContent = 'No metadata for the selected element.';
      panel.appendChild(empty);
      return panel;
    }
    const list = document.createElement('dl');
    list.className = 'diagram-details-grid';
    entries.forEach(([label, value]) => {
      const dt = document.createElement('dt');
      dt.textContent = label;
      const dd = document.createElement('dd');
      dd.textContent = value;
      list.appendChild(dt);
      list.appendChild(dd);
    });
    panel.appendChild(list);
    return panel;
  }

  function buildLegend(titleText, entries) {
    const fragment = document.createDocumentFragment();
    const toolbar = document.createElement('div');
    toolbar.className = 'diagram-toolbar';
    const label = document.createElement('div');
    label.className = 'diagram-toolbar-label';
    label.textContent = titleText;
    toolbar.appendChild(label);
    fragment.appendChild(toolbar);
    const legend = document.createElement('div');
    legend.className = 'diagram-legend';
    entries.forEach((entry) => {
      const item = document.createElement('div');
      item.className = 'diagram-legend-item';
      const swatch = document.createElement('span');
      swatch.className = 'diagram-legend-swatch';
      var fill = entry.fill || '#ffffff';
      swatch.style.background = fill;
      if (entry.dash) {
        swatch.style.borderStyle = 'dashed';
        swatch.style.borderWidth = '2px';
        swatch.style.borderColor = '#111';
      } else {
        swatch.style.border = '2px solid ' + darkenColor(fill, 0.65);
      }
      const text = document.createElement('span');
      text.textContent = entry.label;
      item.appendChild(swatch);
      item.appendChild(text);
      legend.appendChild(item);
    });
    fragment.appendChild(legend);
    return fragment;
  }

  function cloneNode(node, depth, pathPrefix) {
    const label = node.label || node.id || 'Untitled';
    const id = node.id || pathPrefix + '-node-' + depth;
    const children = Array.isArray(node.children) ? node.children : [];
    return {
      id,
      label,
      depth,
      color: node.color || 'gray',
      href: node.href || null,
      kind: node.kind || null,
      file: node.file || null,
      symbol: node.symbol || null,
      line: node.line || null,
      notes: node.notes || null,
      collapsed: Boolean(node.collapsed),
      children: children.map((child, index) => cloneNode(child, depth + 1, id + '-' + index))
    };
  }

  function visibleChildren(node) {
    return node.collapsed ? [] : node.children;
  }

  function annotateTree(node) {
    const sz = nodeSize(node.label);
    node.lines = sz.lines;
    node.width = sz.width;
    node.height = sz.height;
    const children = visibleChildren(node);
    if (!children.length) {
      node.subtreeHeight = node.height;
      return;
    }
    children.forEach(annotateTree);
    const totalChildren = children.reduce((sum, child) => sum + child.subtreeHeight, 0) + (children.length - 1) * 28;
    node.subtreeHeight = Math.max(node.height, totalChildren);
  }

  function positionTree(node, top) {
    const leftPad = 48;
    const topPad = 48;
    const levelGap = 240;
    node.x = leftPad + node.depth * levelGap;
    const children = visibleChildren(node);
    if (!children.length) {
      node.y = topPad + top + (node.subtreeHeight - node.height) / 2;
      return;
    }
    const totalChildren = children.reduce((sum, child) => sum + child.subtreeHeight, 0) + (children.length - 1) * 28;
    let childTop = top + (node.subtreeHeight - totalChildren) / 2;
    children.forEach((child) => {
      positionTree(child, childTop);
      childTop += child.subtreeHeight + 28;
    });
    const first = children[0];
    const last = children[children.length - 1];
    const center = (first.y + first.height / 2 + last.y + last.height / 2) / 2;
    node.y = center - node.height / 2;
  }

  function collectNodes(node, allNodes) {
    allNodes.push(node);
    visibleChildren(node).forEach((child) => collectNodes(child, allNodes));
  }

  function renderMindmap(block, payload) {
    const body = block.querySelector('.diagram-body');
    if (!body) return;
    const data = payload.data || {};
    const root = cloneNode(buildMindmapTree(data), 0, 'mindmap');
    let activeNodeId = null;

    function render() {
      annotateTree(root);
      positionTree(root, 0);

      const allNodes = [];
      collectNodes(root, allNodes);
      const maxRight = Math.max(...allNodes.map((node) => node.x + node.width)) + 96;
      const maxBottom = Math.max(...allNodes.map((node) => node.y + node.height)) + 48;

      body.innerHTML = '';
      const shell = document.createElement('div');
      shell.className = 'mindmap-shell';
      const index = document.createElement('div');
      index.className = 'mindmap-index';
      index.hidden = data.index === false;
      const main = document.createElement('div');
      main.className = 'mindmap-main';
      const viewport = document.createElement('div');
      viewport.className = 'mindmap-viewport';
      const detailsHost = document.createElement('div');
      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      svg.setAttribute('class', 'mindmap-canvas');
      svg.setAttribute('width', String(maxRight));
      svg.setAttribute('height', String(maxBottom));
      svg.setAttribute('viewBox', '0 0 ' + maxRight + ' ' + maxBottom);
      const mmDefs = document.createElementNS('http://www.w3.org/2000/svg', 'defs');
      const mmFilter = document.createElementNS('http://www.w3.org/2000/svg', 'filter');
      mmFilter.setAttribute('id', 'mm-node-shadow');
      mmFilter.setAttribute('x', '-10%'); mmFilter.setAttribute('y', '-20%');
      mmFilter.setAttribute('width', '120%'); mmFilter.setAttribute('height', '140%');
      const mmShadow = document.createElementNS('http://www.w3.org/2000/svg', 'feDropShadow');
      mmShadow.setAttribute('dx', '1'); mmShadow.setAttribute('dy', '2');
      mmShadow.setAttribute('stdDeviation', '2');
      mmShadow.setAttribute('flood-color', '#000'); mmShadow.setAttribute('flood-opacity', '0.1');
      mmFilter.appendChild(mmShadow); mmDefs.appendChild(mmFilter); svg.appendChild(mmDefs);

      const nodeElements = new Map();
      const indexButtons = new Map();

      main.appendChild(buildLegend('Legend', [
        { label: 'Blue — info', fill: colorForNode({ color: 'blue' }) },
        { label: 'Green — done', fill: colorForNode({ color: 'green' }) },
        { label: 'Red — blocker', fill: colorForNode({ color: 'red' }) },
        { label: 'Amber — in progress', fill: colorForNode({ color: 'amber' }) },
        { label: 'Purple — decision', fill: colorForNode({ color: 'purple' }) }
      ]));

      function renderEdges(parent) {
        visibleChildren(parent).forEach((child) => {
          const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
          const startX = parent.x + parent.width;
          const startY = parent.y + parent.height / 2;
          const endX = child.x;
          const endY = child.y + child.height / 2;
          const controlX = startX + (endX - startX) * 0.45;
          path.setAttribute('class', 'mindmap-edge');
          path.setAttribute('d', 'M ' + startX + ' ' + startY + ' C ' + controlX + ' ' + startY + ', ' + (endX - 40) + ' ' + endY + ', ' + endX + ' ' + endY);
          path.style.stroke = darkenColor(colorForNode(child), 0.65);
          svg.appendChild(path);
          renderEdges(child);
        });
      }

      function renderNodes(node) {
        const group = document.createElementNS('http://www.w3.org/2000/svg', 'g');
        group.setAttribute('class', 'mindmap-node');
        group.setAttribute('data-node-id', node.id);
        const rect = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
        rect.setAttribute('x', String(node.x));
        rect.setAttribute('y', String(node.y));
        rect.setAttribute('rx', '12');
        rect.setAttribute('ry', '12');
        rect.setAttribute('width', String(node.width));
        rect.setAttribute('height', String(node.height));
        var mmNodeColor = colorForNode(node);
        rect.style.fill = mmNodeColor;
        rect.style.stroke = darkenColor(mmNodeColor, 0.65);
        rect.setAttribute('filter', 'url(#mm-node-shadow)');
        const mmLines = node.lines || splitLabel(node.label);
        const mmTextY = Math.round(node.y + node.height / 2 + 7 - (mmLines.length - 1) * 11);
        const text = renderLabelText(svg, mmLines, node.x + node.width / 2, mmTextY, 22);
        text.setAttribute('class', 'mindmap-node-text');
        group.appendChild(rect);
        if (node.kind) {
          const badge = document.createElementNS('http://www.w3.org/2000/svg', 'g');
          badge.setAttribute('class', 'mindmap-node-badge');
          const badgeRect = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
          const badgeWidth = 18 + badgeText(node.kind).length * 7;
          badgeRect.setAttribute('x', String(node.x + node.width - badgeWidth - 12));
          badgeRect.setAttribute('y', String(node.y + 8));
          badgeRect.setAttribute('width', String(badgeWidth));
          badgeRect.setAttribute('height', '16');
          badgeRect.setAttribute('rx', '8');
          badgeRect.setAttribute('ry', '8');
          const badgeLabel = document.createElementNS('http://www.w3.org/2000/svg', 'text');
          badgeLabel.setAttribute('x', String(node.x + node.width - badgeWidth / 2 - 12));
          badgeLabel.setAttribute('y', String(node.y + 20));
          badgeLabel.textContent = badgeText(node.kind);
          badge.appendChild(badgeRect);
          badge.appendChild(badgeLabel);
          group.appendChild(badge);
        }
        group.appendChild(text);
        group.addEventListener('click', () => focusNode(node.id));
        svg.appendChild(group);
        nodeElements.set(node.id, { element: group, node });
        if (node.children.length) {
          const toggle = document.createElementNS('http://www.w3.org/2000/svg', 'g');
          toggle.setAttribute('class', 'mindmap-node-toggle');
          const toggleRect = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
          const toggleX = node.x + node.width - 20;
          const toggleY = node.y - 8;
          toggleRect.setAttribute('x', String(toggleX));
          toggleRect.setAttribute('y', String(toggleY));
          toggleRect.setAttribute('width', '18');
          toggleRect.setAttribute('height', '18');
          toggleRect.setAttribute('rx', '4');
          toggleRect.setAttribute('ry', '4');
          const toggleText = document.createElementNS('http://www.w3.org/2000/svg', 'text');
          toggleText.setAttribute('x', String(toggleX + 9));
          toggleText.setAttribute('y', String(toggleY + 14));
          toggleText.textContent = node.collapsed ? '+' : '−';
          toggle.appendChild(toggleRect);
          toggle.appendChild(toggleText);
          toggle.addEventListener('click', (event) => {
            event.stopPropagation();
            node.collapsed = !node.collapsed;
            activeNodeId = node.id;
            render();
          });
          svg.appendChild(toggle);
        }
        visibleChildren(node).forEach(renderNodes);
      }

      function focusNode(nodeId) {
        activeNodeId = nodeId;
        nodeElements.forEach((entry, id) => {
          entry.element.classList.toggle('is-active', id === nodeId);
        });
        indexButtons.forEach((button, id) => {
          button.classList.toggle('is-active', id === nodeId);
        });
        const entry = nodeElements.get(nodeId);
        if (!entry) return;
        const centerX = entry.node.x + entry.node.width / 2;
        const centerY = entry.node.y + entry.node.height / 2;
        viewport.scrollTo({
          left: Math.max(centerX - viewport.clientWidth / 2, 0),
          top: Math.max(centerY - viewport.clientHeight / 2, 0),
          behavior: 'smooth'
        });
        detailsHost.innerHTML = '';
        detailsHost.appendChild(buildDetailsPanel(entry.node.label, metadataEntries(entry.node)));
        if (entry.node.href) {
          window.location.href = entry.node.href;
        } else if (window.location.hash !== '#' + entry.node.id) {
          history.replaceState(null, '', '#' + entry.node.id);
        }
      }

      function buildIndex(node) {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'mindmap-index-item';
        button.textContent = node.label;
        button.style.paddingLeft = (10 + node.depth * 18) + 'px';
        button.addEventListener('click', () => focusNode(node.id));
        index.appendChild(button);
        indexButtons.set(node.id, button);
        visibleChildren(node).forEach(buildIndex);
      }

      renderEdges(root);
      renderNodes(root);
      visibleChildren(root).forEach(buildIndex);
      viewport.appendChild(svg);
      main.appendChild(viewport);
      main.appendChild(detailsHost);
      shell.appendChild(index);
      shell.appendChild(main);
      body.appendChild(shell);

      const defaultNodeId = visibleChildren(root)[0] ? visibleChildren(root)[0].id : root.id;
      const initialId = nodeElements.has(activeNodeId)
        ? activeNodeId
        : (window.location.hash ? window.location.hash.slice(1) : defaultNodeId);
      focusNode(nodeElements.has(initialId) ? initialId : defaultNodeId);
    }

    render();
  }

  function graphFill(kind) {
    const palette = {
      module: '#c8dcf8',
      file: '#b8eecb',
      function: '#d8d0f8',
      service: '#fde68a',
      client: '#f9c4c4',
      backend: '#cdd1d8',
      tool: '#e4e6ea'
    };
    return palette[(kind || '').toLowerCase()] || '#f5f5f5';
  }

  async function renderGraph(block, payload) {
    const body = block.querySelector('.diagram-body');
    if (!body) return;
    const data = payload.data || {};
    const layout = data.layout || {};
    const nodes = Array.isArray(data.nodes) ? data.nodes : [];
    const edges = Array.isArray(data.edges) ? data.edges : [];
    if (!nodes.length) {
      renderUnsupported(block, payload);
      return;
    }
    const elk = createElk();
    const graph = {
      id: 'root',
      layoutOptions: {
        'elk.algorithm': layout.algorithm || 'layered',
        'elk.direction': layout.direction || 'RIGHT',
        'elk.spacing.nodeNode': String(layout.node_spacing || 48),
        'elk.layered.spacing.nodeNodeBetweenLayers': String(layout.layer_spacing || 80)
      },
      children: nodes.map((node) => {
        const sz = nodeSize(node.label || node.id || 'Untitled');
        var h = sz.height;
        if (node.kind) h += 20;
        return {
        id: node.id,
        width: sz.width,
        height: h,
        labels: [{ text: node.label || node.id || 'Untitled' }],
        data: node
        };
      }),
      edges: edges.map((edge, index) => ({
        id: edge.id || 'edge-' + index,
        sources: [edge.from],
        targets: [edge.to],
        labels: edge.label ? [{ text: edge.label }] : []
      }))
    };

    let laidOut;
    try {
      laidOut = await elk.layout(graph);
    } catch (error) {
      renderError(block, 'Graph layout failed', String(error));
      return;
    }

    body.innerHTML = '';
    const shell = document.createElement('div');
    shell.className = 'graph-shell';
    const viewport = document.createElement('div');
    viewport.className = 'graph-viewport';
    const detailsHost = document.createElement('div');
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('class', 'graph-canvas');
    const width = Math.max((laidOut.width || 0) + 96, 800);
    const height = Math.max((laidOut.height || 0) + 96, 520);
    svg.setAttribute('width', String(width));
    svg.setAttribute('height', String(height));
    svg.setAttribute('viewBox', '0 0 ' + width + ' ' + height);
    const defs = document.createElementNS('http://www.w3.org/2000/svg', 'defs');
    const marker = document.createElementNS('http://www.w3.org/2000/svg', 'marker');
    marker.setAttribute('id', 'graph-arrow');
    marker.setAttribute('markerWidth', '10');
    marker.setAttribute('markerHeight', '10');
    marker.setAttribute('refX', '8');
    marker.setAttribute('refY', '3');
    marker.setAttribute('orient', 'auto');
    const arrow = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    arrow.setAttribute('d', 'M0,0 L0,6 L9,3 z');
    arrow.setAttribute('fill', '#666');
    marker.appendChild(arrow);
    defs.appendChild(marker);
    const gFilter = document.createElementNS('http://www.w3.org/2000/svg', 'filter');
    gFilter.setAttribute('id', 'g-node-shadow');
    gFilter.setAttribute('x', '-10%'); gFilter.setAttribute('y', '-20%');
    gFilter.setAttribute('width', '120%'); gFilter.setAttribute('height', '140%');
    const gShadow = document.createElementNS('http://www.w3.org/2000/svg', 'feDropShadow');
    gShadow.setAttribute('dx', '1'); gShadow.setAttribute('dy', '2');
    gShadow.setAttribute('stdDeviation', '2');
    gShadow.setAttribute('flood-color', '#000'); gShadow.setAttribute('flood-opacity', '0.1');
    gFilter.appendChild(gShadow); defs.appendChild(gFilter);
    svg.appendChild(defs);
    const nodeMap = new Map();
    const active = { id: null };
    const sourceNodeMap = new Map(nodes.map((node) => [node.id, node]));

    shell.appendChild(buildLegend('Legend', [
      { label: 'Tool', fill: graphFill('tool') },
      { label: 'Backend', fill: graphFill('backend') },
      { label: 'Client', fill: graphFill('client') },
      { label: 'Service', fill: graphFill('service') },
      { label: 'Dashed edge — semantic', fill: '#f5f5f5', dash: edgePattern('invokes') }
    ]));

    function focusNode(nodeId) {
      active.id = nodeId;
      nodeMap.forEach((entry, id) => {
        entry.classList.toggle('is-active', id === nodeId);
      });
      const node = laidOut.children.find((item) => item.id === nodeId);
      if (!node) return;
      const metadata = sourceNodeMap.get(nodeId) || {};
      detailsHost.innerHTML = '';
      detailsHost.appendChild(buildDetailsPanel(metadata.label || nodeId, metadataEntries({ id: nodeId, ...metadata })));
      viewport.scrollTo({
        left: Math.max(node.x + node.width / 2 - viewport.clientWidth / 2, 0),
        top: Math.max(node.y + node.height / 2 - viewport.clientHeight / 2, 0),
        behavior: 'smooth'
      });
      history.replaceState(null, '', '#' + nodeId);
    }

    (laidOut.edges || []).forEach((edge) => {
      const section = edge.sections && edge.sections[0];
      if (!section) return;
      const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
      const points = [section.startPoint].concat(section.bendPoints || [], [section.endPoint]);
      const d = points.map((point, index) => (index === 0 ? 'M ' : 'L ') + point.x + ' ' + point.y).join(' ');
      path.setAttribute('class', 'graph-edge');
      path.setAttribute('d', d);
      path.setAttribute('marker-end', 'url(#graph-arrow)');
      const sourceEdge = edges.find((item, index) => (item.id || 'edge-' + index) === edge.id);
      const edgeDash = edgePattern(sourceEdge && sourceEdge.kind ? sourceEdge.kind : edge.labels && edge.labels[0] ? edge.labels[0].text : '');
      if (edgeDash) path.setAttribute('stroke-dasharray', edgeDash);
      svg.appendChild(path);
      if (edge.labels && edge.labels[0]) {
        const label = edge.labels[0];
        const text = document.createElementNS('http://www.w3.org/2000/svg', 'text');
        text.setAttribute('class', 'graph-edge-label');
        text.setAttribute('x', String(label.x + label.width / 2));
        text.setAttribute('y', String(label.y + label.height / 2));
        text.textContent = label.text;
        svg.appendChild(text);
      }
    });

    (laidOut.children || []).forEach((node) => {
      const group = document.createElementNS('http://www.w3.org/2000/svg', 'g');
      group.setAttribute('class', 'graph-node');
      group.setAttribute('data-node-id', node.id);
      const rect = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
      rect.setAttribute('x', String(node.x));
      rect.setAttribute('y', String(node.y));
      rect.setAttribute('width', String(node.width));
      rect.setAttribute('height', String(node.height));
      rect.setAttribute('rx', '12');
      rect.setAttribute('ry', '12');
      const sourceNode = sourceNodeMap.get(node.id) || {};
      var nodeColor = sourceNode.color ? colorForNode(sourceNode) : graphFill(sourceNode.kind);
      rect.style.fill = nodeColor;
      rect.style.stroke = darkenColor(nodeColor, 0.65);
      rect.setAttribute('filter', 'url(#g-node-shadow)');
      const gLines = splitLabel(sourceNode.label || node.id);
      const badgeOffset = sourceNode.kind ? 10 : 0;
      const gTextY = Math.round(node.y + node.height / 2 + 7 + badgeOffset - (gLines.length - 1) * 11);
      const text = renderLabelText(svg, gLines, node.x + node.width / 2, gTextY, 22);
      group.appendChild(rect);
      if (sourceNode.kind) {
        const badgeLabel = document.createElementNS('http://www.w3.org/2000/svg', 'text');
        badgeLabel.setAttribute('class', 'graph-node-kind');
        badgeLabel.setAttribute('x', String(node.x + node.width / 2));
        badgeLabel.setAttribute('y', String(node.y + 18));
        badgeLabel.textContent = badgeText(sourceNode.kind);
        group.appendChild(badgeLabel);
      }
      group.appendChild(text);
      group.addEventListener('click', () => focusNode(node.id));
      svg.appendChild(group);
      nodeMap.set(node.id, group);
    });

    viewport.appendChild(svg);
    shell.appendChild(viewport);
    shell.appendChild(detailsHost);
    body.appendChild(shell);
    const initialId = window.location.hash ? window.location.hash.slice(1) : nodes[0].id;
    focusNode(nodeMap.has(initialId) ? initialId : nodes[0].id);
  }

  function renderUnsupported(block, payload) {
    const body = block.querySelector('.diagram-body');
    if (!body) return;
    body.innerHTML = '<div class="diagram-unsupported">Unsupported diagram kind: ' + escapeHtml(payload.kind || 'unknown') + '</div>';
    if (payload.source) {
      body.insertAdjacentHTML('beforeend', '<pre class="diagram-source">' + escapeHtml(payload.source) + '</pre>');
    }
  }

  async function renderDiagram(block) {
    const payload = parsePayload(block);
    if (!payload) return;
    if (payload.kind === 'mermaid') {
      await renderMermaid(block, payload);
      return;
    }
    if (payload.kind === 'mindmap') {
      renderMindmap(block, payload);
      return;
    }
    if (payload.kind === 'graph') {
      await renderGraph(block, payload);
      return;
    }
    renderUnsupported(block, payload);
  }

  async function init() {
    const blocks = Array.from(document.querySelectorAll('.diagram-block'));
    for (const block of blocks) {
      await renderDiagram(block);
    }
  }

  document.addEventListener('DOMContentLoaded', init);
})();
"#;

const ELEMENT_MAP_JS: &str = r#"
(function() {
  var PALETTE = ['#e74c3c','#2196f3','#4caf50','#ff9800','#9c27b0','#009688'];
  var _groups = [];        // tap-link groups: [{id, color, items:[{i,tag,text}]}]
  var _strokeGroups = [];  // stroke-proximity groups: [{color, elementIndices:[number]}]
  var _activeId = null;
  var _nextId = 0;
  var _popup = null;

  var QUERY = 'h1, h2, h3, p, li, pre, blockquote, td, th, table, ul, ol, .diagram-block, img';

  function slugify(s) {
    return 's-' + s.trim().toLowerCase().replace(/[^\w\s-]/g, '').replace(/\s+/g, '-').replace(/-+/g, '-').replace(/^-|-$/g, '');
  }

  function buildMap() {
    window.__einkElementMap = [];
    var content = document.getElementById('content');
    if (!content) return;
    // Assign stable IDs to headings that don't already have one
    content.querySelectorAll('h1, h2, h3, h4, h5, h6').forEach(function(el) {
      if (!el.id) el.id = slugify(el.textContent);
    });
    var currentSection = null;
    content.querySelectorAll(QUERY).forEach(function(el, i) {
      var r = el.getBoundingClientRect();
      var sy = window.pageYOffset || 0;
      var sx = window.pageXOffset || 0;
      var tag = el.tagName;
      if (tag === 'H1' || tag === 'H2' || tag === 'H3') currentSection = el.id || null;
      window.__einkElementMap.push({
        i: i, tag: tag, id: el.id || null, section: currentSection,
        t: r.top + sy, b: r.bottom + sy, l: r.left + sx, r: r.right + sx,
        text: el.textContent.substring(0, 150).replace(/\s+/g, ' ').trim()
      });
    });
  }

  function getEl(i) {
    var content = document.getElementById('content');
    if (!content) return null;
    return content.querySelectorAll(QUERY)[i] || null;
  }

  function groupOf(i) {
    for (var g = 0; g < _groups.length; g++) {
      for (var k = 0; k < _groups[g].items.length; k++) {
        if (_groups[g].items[k].i === i) return _groups[g];
      }
    }
    return null;
  }

  function strokeGroupColorFor(i) {
    for (var s = 0; s < _strokeGroups.length; s++) {
      if (_strokeGroups[s].elementIndices.indexOf(i) >= 0) return _strokeGroups[s].color;
    }
    return null;
  }

  function nextColor() {
    var used = {};
    _groups.forEach(function(g) { used[g.color] = true; });
    _strokeGroups.forEach(function(g) { used[g.color] = true; });
    for (var c = 0; c < PALETTE.length; c++) {
      if (!used[PALETTE[c]]) return PALETTE[c];
    }
    return PALETTE[_nextId % PALETTE.length];
  }

  function nextStrokeColor() {
    var used = {};
    _groups.forEach(function(g) { used[g.color] = true; });
    _strokeGroups.forEach(function(g) { used[g.color] = true; });
    for (var c = 0; c < PALETTE.length; c++) {
      if (!used[PALETTE[c]]) return PALETTE[c];
    }
    return PALETTE[_strokeGroups.length % PALETTE.length];
  }

  function hexToRgba(hex, alpha) {
    var r = parseInt(hex.slice(1, 3), 16);
    var g = parseInt(hex.slice(3, 5), 16);
    var b = parseInt(hex.slice(5, 7), 16);
    return 'rgba(' + r + ',' + g + ',' + b + ',' + alpha + ')';
  }

  // --- Tap-link styling (solid outline) ---

  function applyTapStyle(el, group, pulse) {
    var sb = el.querySelector('.eink-stroke-badge');
    if (sb) sb.parentNode.removeChild(sb);
    el.style.outline = '4px solid ' + group.color;
    el.style.outlineOffset = '3px';
    el.style.backgroundColor = hexToRgba(group.color, 0.08);
    el.style.position = 'relative';
    el.dataset.linkGroupId = String(group.id);
    el.classList.toggle('eink-link-pulse', !!pulse);
    var badge = el.querySelector('.eink-link-badge');
    if (badge) badge.parentNode.removeChild(badge);
    badge = document.createElement('span');
    badge.className = 'eink-link-badge';
    badge.style.background = group.color;
    badge.textContent = group.items.length > 1 ? String(group.items.length) : '\u2295';
    el.appendChild(badge);
  }

  function clearTapStyle(el, elementIndex) {
    el.style.outline = '';
    el.style.outlineOffset = '';
    el.style.backgroundColor = '';
    delete el.dataset.linkGroupId;
    el.classList.remove('eink-link-pulse');
    var b = el.querySelector('.eink-link-badge');
    if (b) b.parentNode.removeChild(b);
    // Restore stroke-link styling if applicable
    var sc = strokeGroupColorFor(elementIndex);
    if (sc) applyStrokeStyle(el, sc);
  }

  // --- Stroke-link styling (left border) ---

  function applyStrokeStyle(el, color) {
    if (el.dataset.linkGroupId) return;
    el.style.borderLeft = '6px solid ' + color;
    el.style.paddingLeft = '8px';
    el.style.backgroundColor = hexToRgba(color, 0.08);
    el.style.position = 'relative';
    var badge = el.querySelector('.eink-stroke-badge');
    if (!badge) {
      badge = document.createElement('span');
      badge.className = 'eink-link-badge eink-stroke-badge';
      badge.style.background = color;
      badge.style.borderStyle = 'solid';
      badge.textContent = '\u270f';
      el.appendChild(badge);
    } else {
      badge.style.background = color;
    }
  }

  function clearStrokeStyle(el) {
    if (el.dataset.linkGroupId) return;
    el.style.borderLeft = '';
    el.style.paddingLeft = '';
    el.style.backgroundColor = '';
    var badge = el.querySelector('.eink-stroke-badge');
    if (badge) badge.parentNode.removeChild(badge);
  }

  // --- Group management helpers ---

  function refreshGroup(group) {
    group.items.forEach(function(item) {
      var el = getEl(item.i);
      if (el) applyTapStyle(el, group, false);
    });
  }

  function showPopup(html, docX, docY) {
    if (_popup && _popup.parentNode) _popup.parentNode.removeChild(_popup);
    var div = document.createElement('div');
    div.className = 'eink-link-popup';
    div.innerHTML = html;
    div.style.left = Math.max(0, docX) + 'px';
    div.style.top = (docY + 16) + 'px';
    document.body.appendChild(div);
    _popup = div;
    setTimeout(function() {
      if (div.parentNode) div.parentNode.removeChild(div);
      if (_popup === div) _popup = null;
    }, 2500);
  }

  // --- Public API ---

  window.__einkFinishLinkGroup = function() {
    if (_activeId === null) return;
    for (var i = 0; i < _groups.length; i++) {
      if (_groups[i].id === _activeId) {
        var g = _groups[i];
        if (g.items.length === 0) {
          _groups.splice(i, 1);
        } else {
          g.items.forEach(function(item) {
            var el = getEl(item.i);
            if (el) el.classList.remove('eink-link-pulse');
          });
        }
        break;
      }
    }
    _activeId = null;
  };

  window.__einkClearLinks = function() {
    _groups.forEach(function(g) {
      g.items.forEach(function(item) {
        var el = getEl(item.i);
        if (el) clearTapStyle(el, item.i);
      });
    });
    _groups = [];
    _activeId = null;
    if (_popup && _popup.parentNode) _popup.parentNode.removeChild(_popup);
    _popup = null;
  };

  // Compute stroke→element associations in JS using bbox overlap.
  // strokes: [[[x,y],...], ...]  (doc coords)
  // explicit: [{strokeIdx, elements:[i,...]}, ...]
  window.__einkComputeStrokeLinks = function(strokes, explicit) {
    var map = window.__einkElementMap || [];
    var MARGIN = 100;  // CSS px expansion around stroke bbox for overlap test

    // strokeIdx → Set of element indices
    var strokeToEls = {};

    // Proximity: bbox-overlap for each non-explicit stroke
    var explicitIdxSet = {};
    (explicit || []).forEach(function(e) { explicitIdxSet[e.strokeIdx] = e.elements; });

    (strokes || []).forEach(function(pts, si) {
      if (explicitIdxSet[si]) {
        strokeToEls[si] = explicitIdxSet[si].slice();
        return;
      }
      if (!pts.length) return;
      var minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
      pts.forEach(function(p) {
        if (p[0] < minX) minX = p[0]; if (p[0] > maxX) maxX = p[0];
        if (p[1] < minY) minY = p[1]; if (p[1] > maxY) maxY = p[1];
      });
      minX -= MARGIN; maxX += MARGIN; minY -= MARGIN; maxY += MARGIN;
      var matched = [];
      for (var k = 0; k < map.length; k++) {
        var e = map[k];
        if (e.r >= minX && e.l <= maxX && e.b >= minY && e.t <= maxY) {
          matched.push(e.i);
        }
      }
      if (matched.length) strokeToEls[si] = matched;
    });

    // Union-find: elements sharing a stroke belong to the same display group
    var parent = {};
    function find(x) {
      if (parent[x] === undefined) parent[x] = x;
      if (parent[x] !== x) parent[x] = find(parent[x]);
      return parent[x];
    }
    function union(a, b) { parent[find(a)] = find(b); }

    Object.keys(strokeToEls).forEach(function(si) {
      var els = strokeToEls[si];
      for (var m = 1; m < els.length; m++) union(els[0], els[m]);
    });

    var grouped = {};
    Object.keys(strokeToEls).forEach(function(si) {
      strokeToEls[si].forEach(function(idx) {
        var root = find(idx);
        if (!grouped[root]) grouped[root] = [];
        if (grouped[root].indexOf(idx) < 0) grouped[root].push(idx);
      });
    });

    var groups = Object.keys(grouped).map(function(k) { return grouped[k]; });
    window.__einkUpdateStrokeLinks(groups);
  };

  window.__einkUpdateStrokeLinks = function(groups) {
    _strokeGroups.forEach(function(g) {
      g.elementIndices.forEach(function(i) {
        var el = getEl(i);
        if (el) clearStrokeStyle(el);
      });
    });
    _strokeGroups = [];

    groups.forEach(function(group) {
      var indices = Array.isArray(group) ? group : [];
      if (!indices.length) return;
      var color = nextStrokeColor();
      _strokeGroups.push({ color: color, elementIndices: indices });
      indices.forEach(function(i) {
        var el = getEl(i);
        if (el) applyStrokeStyle(el, color);
      });
    });
  };

  window.__einkTapElement = function(docX, docY, maxDist) {
    if (maxDist == null) maxDist = 80;
    var map = window.__einkElementMap || [];
    var best = null;
    var bestDist = Infinity;
    for (var i = 0; i < map.length; i++) {
      var e = map[i];
      var dx = Math.max(e.l - docX, 0, docX - e.r);
      var dy = Math.max(e.t - docY, 0, docY - e.b);
      var dist = Math.sqrt(dx * dx + dy * dy);
      if (dist < bestDist) { bestDist = dist; best = e; }
    }
    if (!best || bestDist > maxDist) return JSON.stringify(null);

    var content = document.getElementById('content');
    var el = content.querySelectorAll(QUERY)[best.i];

    var existing = groupOf(best.i);
    if (existing) {
      var names = existing.items.map(function(it) {
        return '\u2022 ' + it.tag + ': ' + it.text.substring(0, 60);
      }).join('<br>');
      showPopup('<strong>Unlinked from group</strong><br>' + names, best.l, best.b);
      existing.items = existing.items.filter(function(it) { return it.i !== best.i; });
      if (el) clearTapStyle(el, best.i);
      if (existing.items.length === 0) {
        _groups = _groups.filter(function(g) { return g.id !== existing.id; });
        if (_activeId === existing.id) _activeId = null;
      } else {
        refreshGroup(existing);
      }
      return JSON.stringify({ i: best.i, tag: best.tag, id: best.id, text: best.text, anchored: false });
    }

    var item = { i: best.i, tag: best.tag, text: best.text.substring(0, 80) };
    var group = null;
    if (_activeId === null) {
      group = { id: _nextId++, color: nextColor(), items: [item] };
      _groups.push(group);
      _activeId = group.id;
    } else {
      for (var j = 0; j < _groups.length; j++) {
        if (_groups[j].id === _activeId) { group = _groups[j]; break; }
      }
      if (!group) {
        group = { id: _nextId++, color: nextColor(), items: [item] };
        _groups.push(group);
        _activeId = group.id;
      } else {
        group.items.push(item);
      }
    }

    var isFirst = group.items.length === 1;
    if (el) applyTapStyle(el, group, isFirst);

    if (!isFirst) {
      group.items.forEach(function(it) {
        var e2 = getEl(it.i);
        if (e2) {
          e2.classList.remove('eink-link-pulse');
          applyTapStyle(e2, group, false);
        }
      });
      showPopup('<strong>Linked \u2014 ' + group.items.length + ' elements</strong>', best.l, best.b);
    }

    return JSON.stringify({ i: best.i, tag: best.tag, id: best.id, text: best.text, anchored: true, color: group.color });
  };

  window.__einkFindElements = function(left, top, right, bottom) {
    var map = window.__einkElementMap || [];
    var matched = [];
    for (var i = 0; i < map.length; i++) {
      var e = map[i];
      if (e.r >= left && e.l <= right && e.b >= top && e.t <= bottom)
        matched.push(e);
    }
    // Prefer inner elements: drop any element that contains another matched element
    var result = matched.filter(function(e) {
      var el = getEl(e.i);
      if (!el) return true;
      for (var j = 0; j < matched.length; j++) {
        if (matched[j].i === e.i) continue;
        var other = getEl(matched[j].i);
        if (other && el.contains(other)) return false;
      }
      return true;
    });
    return JSON.stringify(result.map(function(e) {
      return {i: e.i, tag: e.tag, id: e.id || null, section: e.section || null, text: e.text.substring(0, 80), cx: (e.l + e.r) / 2, cy: (e.t + e.b) / 2};
    }));
  };

  window.__einkFlashGroup = function(elementIndices, colorHex) {
    var r = parseInt(colorHex.slice(1, 3), 16);
    var g = parseInt(colorHex.slice(3, 5), 16);
    var b = parseInt(colorHex.slice(5, 7), 16);
    var els = elementIndices.map(function(i) { return getEl(i); }).filter(Boolean);
    els.forEach(function(el) {
      el.style.outline = '4px solid ' + colorHex;
      el.style.outlineOffset = '3px';
      el.style.backgroundColor = 'rgba(' + r + ',' + g + ',' + b + ',0.15)';
    });
    setTimeout(function() {
      els.forEach(function(el) {
        el.style.outline = '';
        el.style.outlineOffset = '';
        el.style.backgroundColor = '';
      });
    }, 3000);
  };

  window.__einkApplyBindGroups = function(groups) {
    // Clear previous bind styles
    var map = window.__einkElementMap || [];
    for (var i = 0; i < map.length; i++) {
      var el = getEl(map[i].i);
      if (el && el.dataset.bindColor) {
        el.style.outline = '';
        el.style.outlineOffset = '';
        el.style.backgroundColor = '';
        delete el.dataset.bindColor;
      }
    }
    // Apply new bind group styles
    (groups || []).forEach(function(g) {
      var color = g.color || '#e74c3c';
      (g.indices || []).forEach(function(i) {
        var el = getEl(i);
        if (!el || el.dataset.linkGroupId) return;
        el.style.outline = '3px solid ' + color;
        el.style.outlineOffset = '3px';
        el.style.backgroundColor = hexToRgba(color, 0.08);
        el.dataset.bindColor = color;
      });
    });
  };

  window.__einkHighlightAll = function() {
    var map = window.__einkElementMap || [];
    for (var i = 0; i < map.length; i++) {
      var el = getEl(map[i].i);
      if (el) { el.style.outline = '3px dashed #888'; el.style.outlineOffset = '2px'; }
    }
  };

  window.__einkUnhighlightAll = function() {
    var map = window.__einkElementMap || [];
    for (var i = 0; i < map.length; i++) {
      var el = getEl(map[i].i);
      if (el) { el.style.outline = ''; el.style.outlineOffset = ''; }
    }
  };

  document.addEventListener('DOMContentLoaded', buildMap);
  window.addEventListener('resize', buildMap);
})();
"#;

pub fn to_eink_html(markdown: &str, session_id: &str) -> String {
    let content = parse_chunks(markdown)
        .into_iter()
        .map(render_chunk)
        .collect::<Vec<_>>()
        .join("");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=2800, initial-scale=1.0, minimum-scale=0.3, maximum-scale=3.0, user-scalable=yes">
<title>E-Ink Review</title>
<style>{css}</style>
</head>
<body data-session-id="{session_id}">
<div id="content"><button id="toc-toggle">&#9776; Contents</button><nav id="toc" class="collapsed"></nav>{content}</div>
<script src="/assets/diagram/mermaid.min.js"></script>
<script src="/assets/diagram/elk.bundled.js"></script>
<script>{bootstrap_js}</script>
<script>{toc_js}</script>
<script>{element_map_js}</script>
</body>
</html>"#,
        css = EINK_CSS,
        session_id = session_id,
        content = content,
        bootstrap_js = BOOTSTRAP_JS,
        toc_js = TOC_JS,
        element_map_js = ELEMENT_MAP_JS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(markdown: &str) -> String {
        to_eink_html(markdown, "test-id")
    }

    // --- CSS / typography ---
    // CSS tests operate on EINK_CSS directly so the right rule is checked,
    // not just any rule that happens to contain the same value.

    fn css_rule(selector: &str) -> String {
        // Search for the selector at the start of a line to avoid false matches
        // (e.g. "h1 {" inside ".toc-h1 {")
        let needle = format!("\n{selector}");
        let start = EINK_CSS
            .find(needle.as_str())
            .unwrap_or_else(|| panic!("selector '{selector}' not found at line start in CSS"))
            + 1; // skip the leading \n
        let after = &EINK_CSS[start..];
        let open = after.find('{').unwrap();
        let close = after.find('}').unwrap();
        after[open + 1..close].to_string()
    }

    #[test]
    fn body_font_size_is_28px() {
        assert!(
            css_rule("body {").contains("font-size: 28px"),
            "body rule must have font-size: 28px"
        );
    }

    #[test]
    fn heading_sizes_are_correct() {
        assert!(
            css_rule("h1 {").contains("font-size: 42px"),
            "h1 must be 42px"
        );
        assert!(
            css_rule("h2 {").contains("font-size: 34px"),
            "h2 must be 34px"
        );
        assert!(
            css_rule("h3 {").contains("font-size: 28px"),
            "h3 must be 28px"
        );
    }

    #[test]
    fn inline_code_has_strong_border() {
        assert!(
            css_rule("code {").contains("border: 2px solid #999"),
            "inline code needs a visible border"
        );
    }

    #[test]
    fn inline_code_font_size_matches_body() {
        assert!(
            css_rule("code {").contains("font-size: 24px"),
            "inline code should be 24px"
        );
    }

    #[test]
    fn inline_code_renders_code_tag_in_output() {
        let html = render("use `foo::bar` in your code");
        assert!(
            html.contains("<code>foo::bar</code>"),
            "inline code must render as <code> tag"
        );
    }

    // --- TOC ---

    #[test]
    fn toc_shows_for_single_heading() {
        let html = render("# Only Heading\n\nsome text");
        assert!(
            html.contains("id=\"toc-toggle\""),
            "TOC toggle should always appear"
        );
        assert!(
            html.contains("headings.length < 1"),
            "threshold should be < 1"
        );
    }

    #[test]
    fn toc_includes_h1_links() {
        // h1 must get the toc-h1 CSS class in the TOC JS
        let html = render("# Top Level\n\nsome text");
        assert!(
            html.contains("toc-h1"),
            "TOC JS must emit toc-h1 class for h1 elements"
        );
        // The JS must not skip h1
        assert!(
            !html.contains("if (level === 'h1') return;"),
            "h1 must not be excluded from TOC"
        );
    }

    // --- Diagram blocks ---

    #[test]
    fn mermaid_block_renders_diagram_container() {
        let html = render("```mermaid\nflowchart LR\n  A --> B\n```");
        assert!(html.contains("class=\"diagram-block\" data-kind=\"mermaid\""));
        assert!(html.contains("diagram-payload"));
        assert!(html.contains("Rendering Diagram"));
    }

    #[test]
    fn mermaid_sequence_diagram_source_survives_in_payload() {
        // sequenceDiagram is NOT included in the bundled mermaid.min.js (only 2 occurrences
        // vs 70+ for flowchart), so it falls back to raw source on the client.
        // This test documents that the Rust side passes the source through correctly;
        // the rendering limitation is in the JS bundle, not here.
        let source = "sequenceDiagram\n  participant A\n  A->>B: hello";
        let html = render(&format!("```mermaid\n{source}\n```"));
        assert!(html.contains("data-kind=\"mermaid\""));
        // source must be in the JSON payload verbatim
        assert!(
            html.contains("sequenceDiagram"),
            "sequence diagram source must pass through to payload"
        );
    }

    #[test]
    fn mindmap_block_renders_diagram_container() {
        let html = render("```mindmap\nroot: Plan\nnodes:\n  - label: Step\n```");
        assert!(html.contains("class=\"diagram-block\" data-kind=\"mindmap\""));
        assert!(html.contains("Rendering Mind Map"));
    }

    #[test]
    fn graph_block_renders_diagram_container() {
        let html = render("```graph\nnodes:\n  - id: a\n    label: A\nedges: []\n```");
        assert!(html.contains("class=\"diagram-block\" data-kind=\"graph\""));
        assert!(html.contains("Rendering Graph"));
    }

    #[test]
    fn invalid_mindmap_shows_parse_error() {
        // MindMapNode requires `label: String`; omitting it causes a serde_yaml error
        let html = render("```mindmap\nnodes:\n  - id: broken\n```");
        assert!(
            html.contains("Parse Error"),
            "missing label field must produce a parse error"
        );
        assert!(
            html.contains("diagram-error"),
            "error class must be present"
        );
        assert!(
            html.contains("diagram-source"),
            "original source must be shown"
        );
    }

    #[test]
    fn invalid_graph_yaml_shows_parse_error() {
        let html = render("```graph\n: bad: [yaml\n```");
        assert!(html.contains("Parse Error"));
        assert!(html.contains("diagram-error"));
        assert!(html.contains("diagram-source"));
    }

    #[test]
    fn unsupported_fenced_block_is_plain_code() {
        let html = render("```python\nprint('hello')\n```");
        assert!(html.contains("<pre"), "should render as pre block");
        assert!(
            !html.contains("class=\"diagram-block\""),
            "should not be a diagram block"
        );
    }

    // --- Syntax highlighting ---

    #[test]
    fn code_blocks_get_syntax_highlighting() {
        let html = render("```rust\nfn main() {}\n```");
        // syntect wraps in <pre style="..."> with inline colors
        assert!(
            html.contains("<pre style="),
            "should have inline styles from syntect"
        );
    }

    #[test]
    fn solarized_theme_produces_colored_spans() {
        let html = render("```rust\nfn main() { let x = 1; }\n```");
        // Solarized theme produces colored <span> elements
        assert!(html.contains("<span style="), "should have colored spans");
    }

    // --- General structure ---

    #[test]
    fn output_is_valid_html_document() {
        let html = render("# Title\n\nHello");
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<html lang=\"en\">"));
        assert!(html.contains("</html>"));
        assert!(html.contains("/assets/diagram/mermaid.min.js"));
        assert!(html.contains("/assets/diagram/elk.bundled.js"));
    }

    #[test]
    fn markdown_headings_appear_in_body() {
        let html = render("# Title\n## Section\n### Sub");
        assert!(html.contains("<h1>") || html.contains("<h1 "));
        assert!(html.contains("<h2>") || html.contains("<h2 "));
        assert!(html.contains("<h3>") || html.contains("<h3 "));
    }

    #[test]
    fn session_id_embedded_in_html() {
        let html = to_eink_html("hello", "abc123");
        assert!(html.contains("data-session-id=\"abc123\""));
    }

    #[test]
    fn assets_script_tags_present() {
        let html = render("hello");
        assert!(html.contains("src=\"/assets/diagram/mermaid.min.js\""));
        assert!(html.contains("src=\"/assets/diagram/elk.bundled.js\""));
    }

    // --- parse_fence_start ---

    #[test]
    fn parse_fence_start_recognizes_known_kinds() {
        assert_eq!(parse_fence_start("```mermaid"), Some("mermaid"));
        assert_eq!(parse_fence_start("```mindmap"), Some("mindmap"));
        assert_eq!(parse_fence_start("```graph"), Some("graph"));
    }

    #[test]
    fn parse_fence_start_ignores_unknown_kinds() {
        assert_eq!(parse_fence_start("```rust"), None);
        assert_eq!(parse_fence_start("```python"), None);
        assert_eq!(parse_fence_start("```"), None);
        assert_eq!(parse_fence_start("```  "), None);
    }

    #[test]
    fn parse_fence_start_ignores_non_fence_lines() {
        assert_eq!(parse_fence_start("# heading"), None);
        assert_eq!(parse_fence_start("regular text"), None);
        assert_eq!(parse_fence_start(""), None);
        assert_eq!(parse_fence_start("  mermaid"), None);
    }

    #[test]
    fn parse_fence_start_tolerates_surrounding_whitespace() {
        assert_eq!(parse_fence_start("  ```mermaid  "), Some("mermaid"));
        assert_eq!(parse_fence_start("\t```graph"), Some("graph"));
    }

    // --- parse_chunks ---

    #[test]
    fn parse_chunks_returns_single_markdown_chunk_for_plain_text() {
        let chunks = parse_chunks("# Hello\n\nworld");
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], Chunk::Markdown(_)));
    }

    #[test]
    fn parse_chunks_splits_diagram_from_surrounding_markdown() {
        let md = "before\n```mermaid\nA --> B\n```\nafter";
        let chunks = parse_chunks(md);
        assert_eq!(chunks.len(), 3);
        assert!(matches!(&chunks[0], Chunk::Markdown(s) if s.contains("before")));
        assert!(matches!(&chunks[1], Chunk::Diagram { kind, .. } if kind == "mermaid"));
        assert!(matches!(&chunks[2], Chunk::Markdown(s) if s.contains("after")));
    }

    #[test]
    fn parse_chunks_captures_diagram_source_correctly() {
        let md = "```graph\nnodes:\n  - id: a\n```";
        let chunks = parse_chunks(md);
        assert_eq!(chunks.len(), 1);
        if let Chunk::Diagram { kind, source } = &chunks[0] {
            assert_eq!(kind, "graph");
            assert!(source.contains("nodes:"));
        } else {
            panic!("expected Diagram chunk");
        }
    }

    #[test]
    fn parse_chunks_treats_unclosed_fence_as_markdown() {
        let md = "```mermaid\nA --> B\n(no closing fence)";
        let chunks = parse_chunks(md);
        // unclosed fence: all content lands in markdown
        assert!(chunks.iter().all(|c| matches!(c, Chunk::Markdown(_))));
    }

    #[test]
    fn parse_chunks_handles_adjacent_diagram_blocks() {
        let md = "```mermaid\nA\n```\n```graph\nnodes: []\n```";
        let chunks = parse_chunks(md);
        let diagrams: Vec<_> = chunks
            .iter()
            .filter(|c| matches!(c, Chunk::Diagram { .. }))
            .collect();
        assert_eq!(diagrams.len(), 2);
    }

    #[test]
    fn parse_chunks_empty_input_gives_no_chunks() {
        assert!(parse_chunks("").is_empty());
    }

    // --- escape_html ---

    #[test]
    fn escape_html_escapes_all_special_chars() {
        assert_eq!(escape_html("&"), "&amp;");
        assert_eq!(escape_html("<"), "&lt;");
        assert_eq!(escape_html(">"), "&gt;");
        assert_eq!(escape_html("\""), "&quot;");
        assert_eq!(escape_html("'"), "&#39;");
    }

    #[test]
    fn escape_html_handles_combined_input() {
        assert_eq!(
            escape_html("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"
        );
    }

    #[test]
    fn escape_html_leaves_plain_text_unchanged() {
        assert_eq!(escape_html("hello world 123"), "hello world 123");
        assert_eq!(escape_html(""), "");
    }

    // --- diagram_label ---

    #[test]
    fn diagram_label_maps_known_kinds() {
        assert_eq!(diagram_label("mermaid"), "Diagram");
        assert_eq!(diagram_label("mindmap"), "Mind Map");
        assert_eq!(diagram_label("graph"), "Graph");
    }

    #[test]
    fn diagram_label_falls_back_for_unknown() {
        assert_eq!(diagram_label("unknown"), "Diagram");
        assert_eq!(diagram_label(""), "Diagram");
    }

    // --- edge cases ---

    #[test]
    fn empty_input_produces_valid_html() {
        let html = render("");
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
    }

    #[test]
    fn diagram_payload_script_tag_escaped() {
        // </script> inside diagram source must be escaped to prevent breaking the JSON payload block
        let html = render("```mermaid\nA --> B </script> oops\n```");
        // The payload JSON block must contain the escaped form, not the raw closing tag
        assert!(
            html.contains("<\\/script>"),
            "payload must escape </script> as <\\/script>"
        );
    }

    #[test]
    fn render_diagram_error_contains_source() {
        let bad_yaml = ": invalid: [yaml";
        let out = render_diagram("graph", bad_yaml);
        assert!(out.contains("Parse Error"));
        assert!(out.contains("diagram-error"));
        // source should be shown and HTML-escaped
        assert!(out.contains("diagram-source"));
    }

    // --- diff block ---

    #[test]
    fn diff_block_parses_from_fence() {
        assert_eq!(parse_fence_start("```diff"), Some("diff"));
    }

    #[test]
    fn diff_added_lines_get_class() {
        let html = render("```diff\n+new line\n```");
        assert!(
            html.contains("diff-add"),
            "added line should have diff-add class"
        );
        assert!(html.contains("+new line"));
    }

    #[test]
    fn diff_deleted_lines_get_class() {
        let html = render("```diff\n-old line\n```");
        assert!(
            html.contains("diff-del"),
            "deleted line should have diff-del class"
        );
        assert!(html.contains("-old line"));
    }

    #[test]
    fn diff_hunk_headers_get_class() {
        let html = render("```diff\n@@ -1,3 +1,4 @@\n```");
        assert!(html.contains("diff-hunk"));
    }

    #[test]
    fn diff_context_lines_get_class() {
        let html = render("```diff\n unchanged line\n```");
        assert!(html.contains("diff-ctx"));
    }

    #[test]
    fn diff_html_escapes_content() {
        let html = render("```diff\n+<script>evil()</script>\n```");
        // The raw unescaped tag must not appear inside a diff-add div
        assert!(
            !html.contains("class=\"diff-add\"><script>"),
            "raw script tag in diff must be escaped"
        );
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn diff_block_wraps_in_diff_block_class() {
        let html = render("```diff\n+line\n```");
        assert!(html.contains("class=\"diff-block\""));
    }

    #[test]
    fn plain_code_block_does_not_get_diff_styling() {
        let html = render("```bash\n+not a diff\n```");
        // The CSS defines .diff-add but no element should carry class="diff-add"
        assert!(
            !html.contains("class=\"diff-add\""),
            "plain bash block must not get diff styling"
        );
    }
}
