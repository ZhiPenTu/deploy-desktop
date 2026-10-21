import { diffLines } from 'diff';
import { safeText } from '../domain/errors.js';

const normalize = (text) => String(text ?? '').replace(/\r\n/g, '\n');
const linesOf = (text) => text ? text.replace(/\n$/, '').split('\n') : [];

// Keep the original line count so redaction does not shift the diff or its line numbers.
function displayLines(text) {
  let hiddenIndent = null;
  const protectedText = text.replace(
    /-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g,
    (key) => key.split('\n').map(() => '[私钥已隐藏]').join('\n')
  );
  return linesOf(protectedText).map((line) => {
    const indent = line.match(/^\s*/)[0].length;
    if (hiddenIndent !== null && (!line.trim() || indent > hiddenIndent)) {
      return line.trim() ? `${' '.repeat(indent)}[已隐藏]` : line;
    }
    hiddenIndent = null;
    const secret = line.match(/^(\s*(?:-\s*)?["']?[\w.-]*(?:password|passwd|token|secret|authorization|api[_-]?key|private[_-]?key)[\w.-]*["']?\s*[:=]\s*)(.*)$/i);
    if (secret) {
      // Block scalars and nested secret mappings can span several lines.
      if (!secret[2] || secret[2].startsWith('#') || /^[|>][+-]?\d?\s*(?:#.*)?$/.test(secret[2])) hiddenIndent = indent;
      return `${secret[1]}[已隐藏]`;
    }
    return safeText(line);
  });
}

export function buildConfigDiff(current, desired) {
  const oldText = normalize(current);
  const newText = normalize(desired);
  const oldDisplay = displayLines(oldText);
  const newDisplay = displayLines(newText);
  const rows = [];
  let oldIndex = 0;
  let newIndex = 0;

  function append(oldCount, newCount, changed) {
    for (let i = 0; i < Math.max(oldCount, newCount); i += 1) {
      const hasOld = i < oldCount;
      const hasNew = i < newCount;
      rows.push({
        oldLine: hasOld ? oldIndex + 1 : null,
        newLine: hasNew ? newIndex + 1 : null,
        oldText: hasOld ? oldDisplay[oldIndex++] : '',
        newText: hasNew ? newDisplay[newIndex++] : '',
        changed,
        type: !changed ? 'unchanged' : hasOld && hasNew ? 'modified' : hasOld ? 'removed' : 'added'
      });
    }
  }

  const parts = diffLines(oldText, newText);
  for (let index = 0; index < parts.length; index += 1) {
    const part = parts[index];
    if (!part.added && !part.removed) {
      append(part.count, part.count, false);
      continue;
    }
    let removed = 0;
    let added = 0;
    while (index < parts.length && (parts[index].added || parts[index].removed)) {
      if (parts[index].removed) removed += parts[index].count;
      else added += parts[index].count;
      index += 1;
    }
    index -= 1;
    append(removed, added, true);
  }
  const changes = rows.flatMap((row, index) => row.changed && !rows[index - 1]?.changed ? [index] : []);
  return {
    rows,
    changes,
    oldNoNewline: Boolean(oldText && !oldText.endsWith('\n')),
    newNoNewline: Boolean(newText && !newText.endsWith('\n')),
    redacted: oldDisplay.join('\n') !== linesOf(oldText).join('\n') || newDisplay.join('\n') !== linesOf(newText).join('\n')
  };
}

export function serviceDiffRow(rows, service) {
  if (!service) return rows.findIndex((row) => row.changed);
  const header = rows.findIndex((row) => [row.oldText, row.newText].some((line) => {
    const match = line.match(/^(\s+)(?:["']([^"']+)["']|([^:]+)):\s*(?:#.*)?$/);
    return (match?.[2] || match?.[3]) === service;
  }));
  if (header < 0) return rows.findIndex((row) => row.changed);
  const line = rows[header].newLine ? rows[header].newText : rows[header].oldText;
  const indent = line.match(/^\s*/)[0].length;
  for (let index = header; index < rows.length; index += 1) {
    const text = rows[index].newLine ? rows[index].newText : rows[index].oldText;
    if (index > header && text.trim() && !text.trimStart().startsWith('#') && text.match(/^\s*/)[0].length <= indent) break;
    if (rows[index].changed) return index;
  }
  return header;
}
