// Carry local edits forward when another window changes the same record.
export function reconcileEdits(original, draft, latest) {
  const values = {};
  const conflicts = [];
  for (const name of Object.keys(draft)) {
    const edited = draft[name] !== original[name];
    values[name] = edited ? draft[name] : latest[name];
    if (edited && latest[name] !== original[name] && latest[name] !== draft[name]) {
      conflicts.push(name);
    }
  }
  return { values, conflicts };
}

export function showSavedValues(details, conflicts, latest, labels) {
  const list = details.querySelector('dl');
  list.replaceChildren();
  for (const name of conflicts) {
    const label = document.createElement('dt');
    label.textContent = labels[name];
    const value = document.createElement('dd');
    value.textContent = latest[name] || '(empty)';
    list.append(label, value);
  }
  details.hidden = !conflicts.length;
  details.open = Boolean(conflicts.length);
}
