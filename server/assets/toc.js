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
