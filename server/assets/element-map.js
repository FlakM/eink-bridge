(function() {
  var PALETTE = ['#e74c3c','#2196f3','#4caf50','#ff9800','#9c27b0','#009688'];
  var _groups = [];        // tap-link groups: [{id, color, items:[{i,tag,text}]}]
  var _strokeGroups = [];  // stroke-proximity groups: [{color, elementIndices:[number]}]
  var _activeId = null;
  var _nextId = 0;
  var _popup = null;

  // Everything a tap can bind to. A rendered block has to be reachable this way, or a
  // reader cannot annotate it at all, so every block kind names its wrapper here.
  // `tests::every_block_kind_can_be_bound_by_a_tap` holds this to account.
  var QUERY = 'h1, h2, h3, p, li, pre, blockquote, td, th, table, ul, ol, img, .diagram-block, .snippet-block, .steps-block, .diff-block';

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
      // An <img> has no text content; without its alt text a note on a picture reaches
      // the agent as an empty anchor and cannot be told apart from any other block.
      var text = tag === 'IMG'
        ? '[image] ' + (el.getAttribute('alt') || el.getAttribute('src') || '')
        : el.textContent;
      window.__einkElementMap.push({
        i: i, tag: tag, id: el.id || null, section: currentSection,
        t: r.top + sy, b: r.bottom + sy, l: r.left + sx, r: r.right + sx,
        text: text.substring(0, 150).replace(/\s+/g, ' ').trim()
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

  // Where the page is on screen, in the page's own units.
  //
  // The app draws ink through a transform, and the WebView's own scroll/scale properties
  // do not describe the visual viewport once it is zoomed or panned: on this device the
  // visual viewport keeps a constant CSS width while its scale goes from 1 to 1.333 when
  // the tablet is rotated, so ink drawn from the wrong numbers lands 33% off the text it
  // was written on. These four numbers are the mapping: view px per CSS px is
  // (view width in px) / width, and the pan is pageLeft/pageTop times that.
  window.__einkViewport = function() {
    var vv = window.visualViewport;
    if (!vv) return JSON.stringify({ scale: 1, width: window.innerWidth || 0, height: window.innerHeight || 0, pageLeft: window.pageXOffset || 0, pageTop: window.pageYOffset || 0 });
    return JSON.stringify({
      scale: vv.scale, width: vv.width, height: vv.height,
      pageLeft: vv.pageLeft, pageTop: vv.pageTop
    });
  };

  // Keep the element map honest when the page reflows or rotates: the map is in document
  // coordinates, so a rotation that only changes the viewport must not move anything --
  // but a real reflow must.
  window.addEventListener('resize', function() {
    if (window.__einkRebuildElementMap) window.__einkRebuildElementMap();
  });
  window.addEventListener('orientationchange', function() {
    if (window.__einkRebuildElementMap) window.__einkRebuildElementMap();
  });

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

  // --- Step sequences: tap to advance. Degrades to a readable list without this. ---
  function stepBlocks() { return Array.prototype.slice.call(document.querySelectorAll('.steps-block')); }

  function setStep(block, index) {
    var items = block.querySelectorAll('.steps-list li');
    for (var i = 0; i < items.length; i++) {
      items[i].setAttribute('data-state', i < index ? 'done' : (i === index ? 'current' : 'todo'));
    }
    block.setAttribute('data-step', String(index));
  }

  function advanceStep(block) {
    var items = block.querySelectorAll('.steps-list li');
    var current = parseInt(block.getAttribute('data-step') || '0', 10);
    setStep(block, current + 1 >= items.length ? 0 : current + 1);
  }

  window.__einkStepNext = function() {
    var blocks = stepBlocks();
    if (blocks.length) advanceStep(blocks[0]);
  };

  window.__einkStepReset = function() {
    stepBlocks().forEach(function(block) { setStep(block, 0); });
  };

  document.addEventListener('DOMContentLoaded', function() {
    stepBlocks().forEach(function(block) {
      setStep(block, 0);
      var next = block.querySelector('[data-role="step-next"]');
      if (next) next.addEventListener('click', function(event) { event.stopPropagation(); advanceStep(block); });
      var reset = block.querySelector('[data-role="step-reset"]');
      if (reset) reset.addEventListener('click', function(event) { event.stopPropagation(); setStep(block, 0); });
    });
  });

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

  window.__einkRebuildElementMap = buildMap;
  document.addEventListener('DOMContentLoaded', buildMap);
  window.addEventListener('resize', buildMap);
})();
