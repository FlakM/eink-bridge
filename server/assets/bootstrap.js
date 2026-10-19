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
  function edgeLabelDims(text) {
    // Edge labels render at 22px italic bold. Reserve enough box for ELK so
    // labels don't overlap each other or their host edges.
    const lines = String(text || '').split('\n');
    const maxLen = Math.max(1, ...lines.map((l) => l.length));
    return {
      text: String(text || ''),
      width: Math.max(48, maxLen * 13 + 16),
      height: lines.length * 28 + 12
    };
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
      const hint = mermaidHint(String(error), payload.source || '');
      renderError(block, 'Mermaid render failed', String(error) + (hint ? '\n\nHint: ' + hint : '') + '\n\nSource:\n' + (payload.source || ''));
    }
  }

  function mermaidHint(errorMessage, source) {
    // Catch reserved-keyword aliases in sequenceDiagram (case-insensitive).
    // Mermaid's parser treats `and`, `as`, `end`, `loop`, `alt`, `opt`, `par`,
    // `note`, `rect`, `link`, `box`, `par_over`, `critical`, `break`,
    // `activate`, `deactivate` as keywords — using them as participant aliases
    // produces `Expecting ..., got 'and'`-style errors.
    if (!/got '[a-z_]+'/.test(errorMessage)) return '';
    const match = errorMessage.match(/got '([a-z_]+)'/);
    if (!match) return '';
    const token = match[1];
    const reserved = ['and','as','end','loop','alt','opt','par','par_over','note','rect','link','box','critical','break','activate','deactivate','create','destroy','autonumber','title'];
    if (!reserved.includes(token)) return '';
    // Only suggest a rename if that token actually appears as a participant alias.
    const aliasRe = new RegExp('participant\\s+' + token + '\\b', 'i');
    if (!aliasRe.test(source)) return '';
    return "'" + token.toUpperCase() + "' is a reserved mermaid keyword. Rename the participant (try CLI, SRV, TAB, AGT, DB, API, UI, WEB, SVC).";
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
      ['href', node.href || null]
    ].filter(([, value]) => value);
  }

  function buildDetailsPanel(title, entries, notesHtml) {
    const panel = document.createElement('section');
    panel.className = 'diagram-details';
    const heading = document.createElement('div');
    heading.className = 'diagram-details-title';
    heading.textContent = title;
    panel.appendChild(heading);
    if (!entries.length && !notesHtml) {
      const empty = document.createElement('div');
      empty.className = 'eink-muted';
      empty.textContent = 'No metadata for the selected element.';
      panel.appendChild(empty);
      return panel;
    }
    if (entries.length) {
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
    }
    if (notesHtml) {
      const notes = document.createElement('div');
      notes.className = 'diagram-details-notes';
      notes.innerHTML = notesHtml;
      panel.appendChild(notes);
    }
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
      notes_html: node.notes_html || null,
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
      svg.setAttribute('preserveAspectRatio', 'xMidYMid meet');
      svg.removeAttribute('width');
      svg.removeAttribute('height');

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
        const mmLines = node.lines || splitLabel(node.label);
        const mmTextY = Math.round(node.y + node.height / 2 + 7 - (mmLines.length - 1) * 11);
        const text = renderLabelText(svg, mmLines, node.x + node.width / 2, mmTextY, 22);
        text.setAttribute('class', 'mindmap-node-text');
        group.appendChild(rect);
        if (node.kind) {
          const kindLabel = document.createElementNS('http://www.w3.org/2000/svg', 'text');
          kindLabel.setAttribute('class', 'mindmap-node-kind');
          kindLabel.setAttribute('x', String(node.x + node.width / 2));
          kindLabel.setAttribute('y', String(node.y + node.height - 6));
          kindLabel.textContent = node.kind;
          group.appendChild(kindLabel);
        }
        group.appendChild(text);
        if (node.notes_html) {
          const info = document.createElementNS('http://www.w3.org/2000/svg', 'g');
          info.setAttribute('class', 'mindmap-node-info');
          const infoCircle = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
          const infoCx = node.x + 4;
          const infoCy = node.y;
          infoCircle.setAttribute('cx', String(infoCx));
          infoCircle.setAttribute('cy', String(infoCy));
          infoCircle.setAttribute('r', '16');
          const infoText = document.createElementNS('http://www.w3.org/2000/svg', 'text');
          infoText.setAttribute('x', String(infoCx));
          infoText.setAttribute('y', String(infoCy + 7));
          infoText.textContent = 'i';
          info.appendChild(infoCircle);
          info.appendChild(infoText);
          group.appendChild(info);
        }
        group.addEventListener('click', () => focusNode(node.id));
        svg.appendChild(group);
        nodeElements.set(node.id, { element: group, node });
        if (node.children.length) {
          const toggle = document.createElementNS('http://www.w3.org/2000/svg', 'g');
          toggle.setAttribute('class', 'mindmap-node-toggle');
          const toggleRect = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
          const toggleSize = 32;
          const toggleX = node.x + node.width - toggleSize / 2 - 4;
          const toggleY = node.y - toggleSize / 2;
          toggleRect.setAttribute('x', String(toggleX));
          toggleRect.setAttribute('y', String(toggleY));
          toggleRect.setAttribute('width', String(toggleSize));
          toggleRect.setAttribute('height', String(toggleSize));
          toggleRect.setAttribute('rx', '6');
          toggleRect.setAttribute('ry', '6');
          const toggleText = document.createElementNS('http://www.w3.org/2000/svg', 'text');
          toggleText.setAttribute('x', String(toggleX + toggleSize / 2));
          toggleText.setAttribute('y', String(toggleY + toggleSize / 2 + 8));
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
          behavior: 'instant'
        });
        detailsHost.innerHTML = '';
        detailsHost.appendChild(buildDetailsPanel(entry.node.label, metadataEntries(entry.node), entry.node.notes_html));
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
        button.textContent = node.notes_html ? node.label + ' ⓘ' : node.label;
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
        'elk.spacing.edgeNode': String(layout.edge_node_spacing || 36),
        'elk.spacing.edgeEdge': String(layout.edge_edge_spacing || 24),
        'elk.spacing.edgeLabel': String(layout.edge_label_spacing || 16),
        'elk.spacing.labelLabel': '12',
        'elk.layered.spacing.nodeNodeBetweenLayers': String(layout.layer_spacing || 96),
        'elk.layered.spacing.edgeNodeBetweenLayers': String(layout.edge_node_layer_spacing || 48),
        'elk.layered.spacing.edgeEdgeBetweenLayers': String(layout.edge_edge_layer_spacing || 32),
        'elk.layered.edgeLabels.sideSelection': 'SMART_DOWN',
        'elk.edgeLabels.inline': 'false'
      },
      children: nodes.map((node) => {
        const sz = nodeSize(node.label || node.id || 'Untitled');
        return {
        id: node.id,
        width: sz.width,
        height: sz.height,
        labels: [{ text: node.label || node.id || 'Untitled' }],
        data: node
        };
      }),
      edges: edges.map((edge, index) => ({
        id: edge.id || 'edge-' + index,
        sources: [edge.from],
        targets: [edge.to],
        labels: edge.label ? [edgeLabelDims(edge.label)] : []
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

    const childById = new Map((laidOut.children || []).map((n) => [n.id, n]));
    const selfLoops = new Map();
    (laidOut.edges || []).forEach((edge) => {
      if (!edge.sources || !edge.targets) return;
      if (edge.sources[0] !== edge.targets[0]) return;
      const target = childById.get(edge.sources[0]);
      if (!target) return;
      const cx = target.x + target.width / 2;
      const top = target.y;
      const span = Math.max(60, target.width * 0.4);
      const arcLeft = cx - span / 2;
      const arcRight = cx + span / 2;
      const arcTop = top - 64;
      const labelText = edge.labels && edge.labels[0] ? edge.labels[0].text : null;
      selfLoops.set(edge.id, {
        d: 'M ' + arcLeft + ' ' + top + ' C ' + (cx - span) + ' ' + arcTop + ', ' + (cx + span) + ' ' + arcTop + ', ' + arcRight + ' ' + top,
        labelText,
        labelX: cx,
        labelY: arcTop + 18,
        boundLeft: cx - span,
        boundRight: cx + span,
        boundTop: arcTop - 8,
        boundBottom: top
      });
    });

    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    (laidOut.children || []).forEach((n) => {
      minX = Math.min(minX, n.x);
      minY = Math.min(minY, n.y);
      maxX = Math.max(maxX, n.x + n.width);
      maxY = Math.max(maxY, n.y + n.height);
    });
    (laidOut.edges || []).forEach((edge) => {
      if (selfLoops.has(edge.id)) return;
      (edge.sections || []).forEach((sec) => {
        const pts = [sec.startPoint].concat(sec.bendPoints || [], [sec.endPoint]);
        pts.forEach((p) => {
          if (!p) return;
          minX = Math.min(minX, p.x);
          minY = Math.min(minY, p.y);
          maxX = Math.max(maxX, p.x);
          maxY = Math.max(maxY, p.y);
        });
      });
      (edge.labels || []).forEach((lbl) => {
        if (lbl.x == null || lbl.y == null) return;
        minX = Math.min(minX, lbl.x);
        minY = Math.min(minY, lbl.y);
        maxX = Math.max(maxX, lbl.x + (lbl.width || 0));
        maxY = Math.max(maxY, lbl.y + (lbl.height || 0));
      });
    });
    selfLoops.forEach((loop) => {
      minX = Math.min(minX, loop.boundLeft);
      minY = Math.min(minY, loop.boundTop);
      maxX = Math.max(maxX, loop.boundRight);
      maxY = Math.max(maxY, loop.boundBottom);
    });
    if (!isFinite(minX)) { minX = 0; minY = 0; maxX = 0; maxY = 0; }
    const pad = 48;
    const vbX = minX - pad;
    const vbY = minY - pad;
    const vbW = Math.max(maxX - minX + pad * 2, 800);
    const vbH = Math.max(maxY - minY + pad * 2, 520);
    svg.setAttribute('viewBox', vbX + ' ' + vbY + ' ' + vbW + ' ' + vbH);
    svg.setAttribute('preserveAspectRatio', 'xMidYMid meet');
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
    arrow.setAttribute('fill', '#222');
    marker.appendChild(arrow);
    defs.appendChild(marker);
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
      detailsHost.appendChild(buildDetailsPanel(metadata.label || nodeId, metadataEntries({ id: nodeId, ...metadata }), metadata.notes_html || null));
      viewport.scrollTo({
        left: Math.max(node.x - vbX + node.width / 2 - viewport.clientWidth / 2, 0),
        top: Math.max(node.y - vbY + node.height / 2 - viewport.clientHeight / 2, 0),
        behavior: 'smooth'
      });
      history.replaceState(null, '', '#' + nodeId);
    }

    (laidOut.edges || []).forEach((edge) => {
      const sourceEdge = edges.find((item, index) => (item.id || 'edge-' + index) === edge.id);
      const loop = selfLoops.get(edge.id);
      if (loop) {
        const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('class', 'graph-edge');
        path.setAttribute('d', loop.d);
        path.setAttribute('fill', 'none');
        path.setAttribute('marker-end', 'url(#graph-arrow)');
        const edgeDash = edgePattern(sourceEdge && sourceEdge.kind ? sourceEdge.kind : loop.labelText || '');
        if (edgeDash) path.setAttribute('stroke-dasharray', edgeDash);
        svg.appendChild(path);
        if (loop.labelText) {
          const text = document.createElementNS('http://www.w3.org/2000/svg', 'text');
          text.setAttribute('class', 'graph-edge-label');
          text.setAttribute('x', String(loop.labelX));
          text.setAttribute('y', String(loop.labelY));
          text.textContent = loop.labelText;
          svg.appendChild(text);
        }
        return;
      }
      const section = edge.sections && edge.sections[0];
      if (!section) return;
      const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
      const points = [section.startPoint].concat(section.bendPoints || [], [section.endPoint]);
      const d = points.map((point, index) => (index === 0 ? 'M ' : 'L ') + point.x + ' ' + point.y).join(' ');
      path.setAttribute('class', 'graph-edge');
      path.setAttribute('d', d);
      path.setAttribute('marker-end', 'url(#graph-arrow)');
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
      const gLines = splitLabel(sourceNode.label || node.id);
      const gTextY = Math.round(node.y + node.height / 2 + 7 - (gLines.length - 1) * 11);
      const text = renderLabelText(svg, gLines, node.x + node.width / 2, gTextY, 22);
      group.appendChild(rect);
      if (sourceNode.kind) {
        const badgeLabel = document.createElementNS('http://www.w3.org/2000/svg', 'text');
        badgeLabel.setAttribute('class', 'graph-node-kind');
        badgeLabel.setAttribute('x', String(node.x + node.width / 2));
        badgeLabel.setAttribute('y', String(node.y + node.height - 8));
        badgeLabel.textContent = sourceNode.kind;
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
