// HTML escaping and syntax highlighters for unified diffs and Python snippets.

export function escapeHtml(raw) {
  return String(raw ?? '')
    .replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;').replaceAll("'", '&#39;');
}

export function highlightUnifiedDiff(diffText) {
  return String(diffText ?? '').split('\n').map((line) => {
    const cls = line.startsWith('+++') || line.startsWith('---') || line.startsWith('@@')
      ? ' diff-hunk' : line.startsWith('+') ? ' diff-add' : line.startsWith('-') ? ' diff-del' : '';
    return `<span class="diff-line${cls}">${escapeHtml(line)}</span>`;
  }).join('');
}

export function highlightPython(codeText) {
  const kw = /\b(import|from|as|def|return|if|elif|else|for|in|while|with|try|except|True|False|None|and|or|not)\b/g;
  return String(codeText ?? '').split('\n').map((line) => {
    const hashIdx = line.indexOf('#');
    const head = hashIdx >= 0 ? line.slice(0, hashIdx) : line;
    const tail = hashIdx >= 0 ? line.slice(hashIdx) : '';
    let html = escapeHtml(head)
      .replace(/(&quot;[^&]*?&quot;|&#39;[^&]*?&#39;)/g, '<span class="py-str">$1</span>')
      .replace(kw, '<span class="py-kw">$1</span>');
    if (tail) html += `<span class="py-com">${escapeHtml(tail)}</span>`;
    return `<span class="diff-line">${html}</span>`;
  }).join('');
}
