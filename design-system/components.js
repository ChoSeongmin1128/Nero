(function (root) {
  const escape = (value) => String(value ?? "").replace(/[&<>"']/g, (character) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[character]);
  const attributes = (values = {}) => Object.entries(values).map(([name, value]) => {
    if (!/^(?:id|title|role|name|value|tabindex|type|class|aria-[\w-]+|data-[\w-]+)$/.test(name)) throw new Error("Unsupported component attribute: " + name);
    return value == null ? "" : ` ${name}="${escape(value)}"`;
  }).join("");
  const icon = (name) => root.NeroIcons?.(name) || "";
  const phaseName = (phase) => ["ready", "recording", "paused", "finished"].includes(phase) ? phase : "ready";
  function highlightText(value, query = "") {
    const text = String(value ?? ""), term = String(query ?? "").trim();
    if (!term) return escape(text);
    const matches = text.matchAll(new RegExp(term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "giu"));
    let html = "", cursor = 0;
    for (const match of matches) {
      html += escape(text.slice(cursor, match.index)) + `<mark class="search-match">${escape(match[0])}</mark>`;
      cursor = match.index + match[0].length;
    }
    return html + escape(text.slice(cursor));
  }
  function button({ text, html, action, className = "", withIcon = false, disabled = false, attrs = {} } = {}) {
    const content = html == null ? escape(text) : html;
    const classes = [className, withIcon ? "with-icon" : ""].filter(Boolean).join(" ");
    return `<button${attributes({ type: "button", "data-action": action, class: classes, ...attrs })}${disabled ? " disabled" : ""}>${content || ""}</button>`;
  }
  function badge(text, warning = false) { return `<span class="badge${warning ? " warn" : ""}">${escape(text)}</span>`; }
  function status(phase, label) { return `<span class="status-chip ${phaseName(phase)}">${escape(label)}</span>`; }
  function meetingRow({ title, phase, count = 0, date, label, attendeeLabel, action, attrs = {} }) {
    const state = phaseName(phase);
    return button({ action, className: "library-row", attrs,
      html: `<span class="meeting-symbol ${state}">${icon(state === "ready" ? "calendar" : state === "finished" ? "file-text" : "mic")}</span><span class="library-row-main"><strong title="${escape(title)}">${escape(title)}</strong></span><span class="library-row-people" title="${escape(attendeeLabel || (count ? count + "명" : "참석자 미지정"))}">${count ? escape(count) + "명" : "미지정"}</span><span class="library-row-date">${escape(date)}</span>${status(state, label)}${icon("chevron-right")}` });
  }
  function personRow({ name, role = "역할 미지정", initial, statusText = "목소리 미등록" }) {
    return `<div class="person-row"><span class="avatar" aria-hidden="true">${escape(initial || name?.slice(0, 1))}</span><div class="grow person-identity"><strong>${escape(name)}</strong><span class="muted">${escape(role)}</span></div><span class="person-voice-status">${escape(statusText)}</span></div>`;
  }
  function personOption({ name, role = "역할 미지정", initial, checked = false, attrs = {} }) {
    return `<label class="selector-person${checked ? " is-selected" : ""}"><input type="checkbox"${attributes(attrs)}${checked ? " checked" : ""}><span class="avatar" aria-hidden="true">${escape(initial || name?.slice(0, 1))}</span><span class="person-identity"><strong>${escape(name)}</strong><span class="muted">${escape(role)}</span></span></label>`;
  }
  function utterance({ speaker, time, tone = 0, unknown = false, current = false, editing = false, text, contentHTML, timeAction, timeAttrs = {}, actionsHTML = "", corrected = false }) {
    return `<article class="utterance${current ? " current" : ""}${editing ? " editing" : ""}"><div class="speaker-rail"><span class="speaker-avatar ${unknown ? "unknown" : "tone-" + (Math.max(0, tone) % 3)}" aria-hidden="true">${unknown ? "?" : escape(speaker?.slice(0, 1))}</span><strong>${escape(speaker)}</strong>${button({ text: time, action: timeAction, attrs: { "aria-label": time + " 발언", ...timeAttrs } })}${unknown ? badge("이름 미확인", true) : ""}</div><div>${editing ? contentHTML : `<p${current ? ' class="pending"' : ""}>${escape(text)}</p>${corrected ? '<span class="tiny muted">수정됨</span>' : ""}<div class="turn-actions">${actionsHTML}</div>`}</div></article>`;
  }
  function transcriptRow({ index, time, start, text, editing = false, disabled = false, speaker = "", speakerVisible = false, searchQuery = "" }) {
    const label = `${Number(index) + 1}번 발언`;
    const content = editing
      ? `<textarea class="text-input"${attributes({ "data-segment": index, "aria-label": label + " 수정" })} rows="1" spellcheck="false"${disabled ? " readonly" : ""}>${escape(text)}</textarea><div class="transcript-edit-actions">${button({ text: "취소", className: "quiet small", disabled, attrs: { "data-transcript-cancel": index, "aria-label": label + " 수정 취소" } })}${button({ text: "완료", className: "small", disabled, attrs: { "data-transcript-done": index, "aria-label": label + " 수정 완료" } })}</div>`
      : `<p class="transcript-text">${highlightText(text, searchQuery)}</p>`;
    return `<article${attributes({ class: "transcript-row" + (editing ? " is-editing" : "") + (speakerVisible ? " has-speaker" : ""), "data-transcript-row": index, tabindex: -1 })}>${button({ text: time, className: "transcript-time quiet small", attrs: { "data-seek": start, "aria-label": time + " 발언 재생" } })}${speakerVisible ? button({ html: highlightText(speaker || "화자 미지정", searchQuery), className: "transcript-speaker quiet small", disabled, attrs: { "data-speaker-edit": index, "aria-label": label + " 화자 변경: " + (speaker || "화자 미지정"), title: speaker || "화자 미지정" } }) : ""}<div class="transcript-content">${content}</div>${editing ? "" : `<div class="transcript-row-actions">${button({ html: icon("pencil"), className: "quiet small icon", disabled, attrs: { "data-transcript-edit": index, "aria-label": label + " 수정", title: "발언 수정" } })}</div>`}</article>`;
  }
  function transcriptPreview(rows) {
    return `<div class="transcript-preview">${rows.map(({ time, speaker, text }) => `<article class="transcript-preview-row">${time || speaker ? `<div class="transcript-preview-meta">${time ? `<span>${escape(time)}</span>` : ''}${speaker ? `<span>${escape(speaker)}</span>` : ''}</div>` : ''}<p>${escape(text)}</p></article>`).join('')}</div>`;
  }
  function minutes(sections) {
    return `<article class="minutes-document">${sections.map(({ title, text, emptyText = "작성된 내용이 없습니다." }) => `<section><h3>${escape(title)}</h3><div class="minutes-text">${String(text || "").trim() ? escape(text) : `<span class="muted">${escape(emptyText)}</span>`}</div></section>`).join("")}</article>`;
  }
  // `citable`: when given, only groups whose numbers are all in it become citations.
  function summaryDocument(value, { searchQuery = "", streaming = false, citable = null } = {}) {
    const parser = root.NeroMarkdown;
    if (!parser?.parse) throw new Error("Markdown parser is not loaded.");
    const headings = new Set(["회의 요약", "요약", "주요 내용", "논의 내용", "논의 사항", "결정 사항", "결정사항", "합의 사항", "다음 작업", "후속 작업", "할 일", "할일", "액션 아이템", "미결 사항", "확인 필요", "참고 사항"]);
    const term = String(searchQuery ?? "").trim();
    const pattern = term ? new RegExp(term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "giu") : null;
    let searchGroup = 0;
    const references = (text, defined = new Set()) => {
      const tokens = [];
      let cursor = 0;
      // Keep the saved one-based references. Never resolve them against current rows.
      for (const match of text.matchAll(/\[([0-9, \t-]+)\]/g)) {
        const before = text.slice(0, match.index), after = text.slice(match.index + match[0].length);
        if (/(?:\\|!)$/.test(before) || /^(?:\(|\[[^0-9])/.test(after) || defined.has(match[1].replace(/\s+/g, " ").trim())) continue;
        const refs = [];
        let valid = true;
        for (const part of match[1].split(',')) {
          const range = part.trim().match(/^(\d+)(?:\s*-\s*(\d+))?$/);
          const first = Number(range?.[1]), last = Number(range?.[2] ?? range?.[1]);
          if (!range || !Number.isSafeInteger(first) || !Number.isSafeInteger(last) || first < 1 || last < first || last - first >= 32) { valid = false; break; }
          for (let number = first; number <= last; number += 1) {
            if (!refs.includes(number)) refs.push(number);
            if (refs.length > 32) { valid = false; break; }
          }
          if (!valid) break;
        }
        if (!valid || !refs.length || (citable && !refs.every(number => citable.includes(number)))) continue;
        const previous = tokens.at(-1), gap = text.slice(cursor, match.index);
        const merged = previous?.references ? [...new Set([...previous.references, ...refs])] : [];
        if (previous?.references && /^[ \t]*$/.test(gap) && merged.length <= 32) {
          previous.text += gap + match[0]; previous.references = merged;
        } else {
          tokens.push({ text: gap }, { text: match[0], references: refs });
        }
        cursor = match.index + match[0].length;
      }
      tokens.push({ text: text.slice(cursor) });
      return tokens;
    };
    const renderTokens = tokens => {
      const visible = nodes => nodes.map(node => node.children ? visible(node.children) : node.text).join("");
      const ranges = pattern ? [...visible(tokens).matchAll(pattern)].map(match => ({ start: match.index, end: match.index + match[0].length, group: `summary-${++searchGroup}` })) : [];
      let offset = 0, rangeIndex = 0;
      const textHTML = text => {
        if (!text) return "";
        const start = offset, end = start + text.length;
        let html = "", cursor = start;
        while (rangeIndex < ranges.length && ranges[rangeIndex].start < end) {
          const range = ranges[rangeIndex], from = Math.max(start, range.start), to = Math.min(end, range.end);
          html += escape(text.slice(cursor - start, from - start)) + `<mark class="search-match" data-search-group="${range.group}">${escape(text.slice(from - start, to - start))}</mark>`;
          cursor = to;
          if (range.end > end) break;
          rangeIndex += 1;
        }
        offset = end;
        return html + escape(text.slice(cursor - start));
      };
      const render = node => {
        if (node.br) { textHTML(node.text); return "<br>"; }
        if (node.href) return `<a class="summary-link" href="${escape(node.href)}" title="${escape(node.href)}" draggable="false" data-summary-link="">${node.children.map(render).join("")}</a>`;
        if (node.children) return `<${node.tag}>${node.children.map(render).join("")}</${node.tag}>`;
        const start = offset;
        const html = textHTML(node.text);
        if (!node.references) return html;
        const label = node.references.length > 1 ? `근거 ${node.references.length}` : "근거";
        const matches = ranges.filter(range => range.start < offset && range.end > start);
        const visibleLabel = matches.length ? `<mark class="search-match" data-search-group="${matches[0].group}">${label}</mark>` : label;
        const content = `<span aria-hidden="true">${icon("messages-square")}</span><span class="sr-only">${visibleLabel}${html}</span>`;
        return button({ html: content, className: "quiet source-reference", attrs: { "data-summary-citations": node.references.join(","), "aria-pressed": "false", "title": "음성 기록에서 발언 확인", "aria-label": `${node.references.join(", ")}번 발언 확인` } });
      };
      return tokens.map(render).join("");
    };
    // Citations are recognized on the raw text, where escapes and code are still visible,
    // and carried through the parser as private-use placeholders.
    // A per-render nonce keeps model text, even entity-encoded, from forging a placeholder.
    const citations = [], nonce = Math.random().toString(36).slice(2, 10).padEnd(8, "0");
    const placeholder = new RegExp(`\uE000${nonce}([0-9a-z]+)\uE001`, "g");
    const protect = text => {
      // A numeric label with a definition is a link reference, not a citation.
      const defined = new Set([...text.matchAll(/^[ \t]{0,3}\[([0-9, \t-]+)\]:/gm)].map(match => match[1].replace(/\s+/g, " ").trim()));
      let out = "";
      const tokens = references(text, defined);
      tokens.forEach((token, index) => {
        if (!token.references) { out += token.text; return; }
        // Sentence punctuation written after a citation belongs to the sentence: "\uD588\uC2B5\uB2C8\uB2E4 [3]." reads "\uD588\uC2B5\uB2C8\uB2E4.[3]".
        const next = tokens[index + 1], mark = next && !next.references ? next.text.match(/^[.!?,;:\u3002\u3001\uFF01\uFF1F\u2026]+/)?.[0] : "";
        if (mark && /\S$/.test(out.replace(/[ \t]+$/, ""))) {
          out = out.replace(/[ \t]+$/, "");
          if (!out.endsWith(mark)) out += mark;
          next.text = next.text.slice(mark.length);
        }
        // GFM autolinks run until whitespace, so a citation right after a URL would join it.
        if (/(?:https?:\/\/|www\.)\S*$/i.test(out)) out += " ";
        out += `\uE000${nonce}${(citations.push(token) - 1).toString(36)}\uE001`;
      });
      return out;
    };
    const restore = text => String(text ?? "").replace(placeholder, (_, index) => citations[parseInt(index, 36)]?.text ?? "");
    const textTokens = text => {
      const tokens = [];
      let cursor = 0;
      for (const match of String(text ?? "").matchAll(placeholder)) {
        tokens.push(...lines(text.slice(cursor, match.index)));
        const citation = citations[parseInt(match[1], 36)];
        if (citation) tokens.push({ text: citation.text, references: citation.references });
        cursor = match.index + match[0].length;
      }
      tokens.push(...lines(String(text ?? "").slice(cursor)));
      return tokens;
    };
    const lines = text => text.split("\n").flatMap((line, index) => index ? [{ text: "\n", br: true }, { text: line }] : [{ text: line }]).filter(token => token.br || token.text);
    const plain = tokens => tokens.map(token => token.children ? { ...token, children: plain(token.children) } : { text: token.text, br: token.br });
    const safeLink = url => { try { const parsed = new URL(restore(url)); return parsed.protocol === "https:" && !parsed.username && !parsed.password ? parsed.href : ""; } catch { return ""; } };
    const definitions = new Map();
    const inline = nodes => (nodes || []).flatMap(node => {
      switch (node.type) {
        case "text": return textTokens(node.value);
        case "strong": return [{ tag: "strong", children: inline(node.children) }];
        case "emphasis": return [{ tag: "em", children: inline(node.children) }];
        case "delete": return [{ tag: "del", children: inline(node.children) }];
        case "inlineCode": return [{ tag: "code", children: [{ text: restore(node.value) }] }];
        case "break": return [{ text: "\n", br: true }];
        case "link": case "linkReference": {
          const href = safeLink(node.type === "link" ? node.url : definitions.get(node.identifier) || "");
          const children = plain(inline(node.children));
          return href ? [{ tag: "a", href, children }] : children;
        }
        case "image": case "imageReference": return node.alt ? [{ text: restore(node.alt) }] : [];
        case "footnoteReference": return [{ tag: "sup", children: [{ text: `[${node.label ?? node.identifier}]` }] }];
        default: return node.children ? inline(node.children) : node.value ? textTokens(node.value) : [];
      }
    });
    const inlineHTML = nodes => renderTokens(inline(nodes));
    const aligned = new Set(["left", "center", "right"]);
    const block = (node, tight = false) => {
      switch (node.type) {
        case "heading": { const level = Math.min(6, Math.max(3, node.depth + 1)); return `<h${level}>${inlineHTML(node.children)}</h${level}>`; }
        case "paragraph": return tight ? inlineHTML(node.children) : `<p>${inlineHTML(node.children)}</p>`;
        case "list": {
          const tag = node.ordered ? "ol" : "ul", start = node.ordered && Number.isSafeInteger(node.start) ? ` start="${node.start}"` : "";
          return `<${tag}${start}>${node.children.map(item => block(item, !node.spread)).join("")}</${tag}>`;
        }
        case "listItem": {
          const task = typeof node.checked === "boolean" ? `<span class="summary-task" data-checked="${node.checked}" aria-hidden="true"></span><span class="sr-only">${node.checked ? "완료" : "할 일"}</span>` : "";
          return `<li${task ? ' class="summary-task-item"' : ""}>${task}${node.children.map(child => block(child, tight && !node.spread)).join("")}</li>`;
        }
        case "blockquote": return `<blockquote>${node.children.map(child => block(child)).join("")}</blockquote>`;
        case "code": return `<pre><code>${renderTokens([{ text: restore(node.value) }])}</code></pre>`;
        case "thematicBreak": return "<hr>";
        case "table": {
          const cell = (tag, item, index) => `<${tag}${aligned.has(node.align?.[index]) ? ` data-align="${node.align[index]}"` : ""}>${inlineHTML(item.children)}</${tag}>`;
          const [head, ...rows] = node.children;
          const body = rows.length ? `<tbody>${rows.map(row => `<tr>${row.children.map((item, index) => cell("td", item, index)).join("")}</tr>`).join("")}</tbody>` : "";
          return `<div class="summary-table"><table><thead><tr>${(head?.children || []).map((item, index) => cell("th", item, index)).join("")}</tr></thead>${body}</table></div>`;
        }
        case "html": return `<p>${renderTokens(textTokens(node.value))}</p>`;
        case "definition": return "";
        case "footnoteDefinition": return `<div class="summary-footnote"><span class="summary-footnote-label">[${escape(node.label ?? node.identifier)}]</span>${node.children.map(child => block(child, true)).join(" ")}</div>`;
        default: return node.children ? node.children.map(child => block(child)).join("") : "";
      }
    };
    // Model output often marks a section with a bare known title line instead of "#".
    const headingText = line => {
      const visible = line.filter(node => node.type !== "text" || node.value.trim());
      if (visible.length !== 1) return "";
      const [node] = visible;
      const text = node.type === "text" ? node.value : node.type === "strong" && node.children.length === 1 && node.children[0].type === "text" ? node.children[0].value : "";
      const title = text.trim().replace(/[:：]$/, "").trim();
      return headings.has(title) ? title : "";
    };
    const promote = nodes => nodes.flatMap(node => {
      if (node.type === "blockquote") return [{ ...node, children: promote(node.children) }];
      if (node.type !== "paragraph") return [node];
      const rows = [[]];
      for (const child of node.children) {
        if (child.type === "text" && child.value.includes("\n")) child.value.split("\n").forEach((part, index) => { if (index) rows.push([]); if (part) rows.at(-1).push({ ...child, value: part }); });
        else if (child.type === "break") rows.push([]);
        else rows.at(-1).push(child);
      }
      if (!rows.some(headingText)) return [node];
      const out = [];
      let pending = [];
      const flush = () => { if (pending.length) out.push({ type: "paragraph", children: pending.flatMap((row, index) => index ? [{ type: "text", value: "\n" }, ...row] : row) }); pending = []; };
      for (const row of rows) {
        const title = headingText(row);
        if (title) { flush(); out.push({ type: "heading", depth: 2, children: [{ type: "text", value: title }] }); }
        else if (row.some(item => item.type !== "text" || item.value.trim())) pending.push(row);
      }
      flush();
      return out;
    });
    const tree = parser.parse(protect(String(value ?? "").replace(/\r\n?/g, "\n").replace(/[\uE000\uE001]/g, "")), { streaming });
    (function collect(node) { if (node.type === "definition" && !definitions.has(node.identifier)) definitions.set(node.identifier, node.url); (node.children || []).forEach(collect); })(tree);
    return `<article class="summary-document">${promote(tree.children).map(child => block(child)).join("")}</article>`;
  }
  function dialogContent({ title, titleId = "modal-title", bodyHTML = "", actionsHTML = "" }) {
    return `<h2${attributes({ id: titleId })}>${escape(title)}</h2>${bodyHTML}${actionsHTML ? `<div class="modal-footer"><span></span><div class="row">${actionsHTML}</div></div>` : ""}`;
  }
  const api = { escape, highlightText, button, badge, status, meetingRow, personRow, personOption, utterance, transcriptRow, transcriptPreview, minutes, summaryDocument, dialogContent };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.NeroUI = api;
})(typeof globalThis === "object" ? globalThis : this);
