  // Tap a `path:line` reference to read that code here, without leaving the page.
  // The block is fetched once and kept, so the second tap is instant.
  document.addEventListener('click', function (event) {
    var target = event.target && event.target.closest ? event.target.closest('.coderef') : null;
    if (!target) return;
    var ref = target.getAttribute('data-ref');
    var existing = target.nextElementSibling;
    var isOurs = existing && existing.classList && existing.classList.contains('coderef-wrap');
    if (isOurs) {
      var open = target.getAttribute('data-open') === '1';
      target.setAttribute('data-open', open ? '0' : '1');
      existing.style.display = open ? 'none' : 'block';
      return;
    }
    target.setAttribute('data-open', '1');
    var wrap = document.createElement('div');
    wrap.className = 'coderef-wrap';
    wrap.innerHTML = '<p class="coderef-note">reading ' + ref + '...</p>';
    target.parentNode.insertBefore(wrap, target.nextSibling);
    var session = document.body.getAttribute('data-session-id') || '';
    fetch('/api/sessions/' + encodeURIComponent(session) + '/source?ref=' + encodeURIComponent(ref))
      .then(function (response) {
        if (!response.ok) throw new Error(response.status === 404 ? 'not found' : 'unavailable');
        return response.json();
      })
      .then(function (body) {
        // The server renders it: the same highlighter, palette, caption and cited-line
        // mark as every other code block on the page. Inserting the lines as text here
        // made the expansion the only uncoloured code on the tablet.
        wrap.innerHTML = body.html;
      })
      .catch(function (error) {
        // Say so rather than showing an empty block: the worktree may be gone.
        wrap.innerHTML = '<p class="coderef-note">' + ref + ' is not available here (' + error.message + ')</p>';
      });
  });
