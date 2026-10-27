(function (root) {
  const escape = (value) => String(value ?? "").replace(/[&<>"']/g, (character) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[character]);
  const attributes = (values = {}) => Object.entries(values).map(([name, value]) => {
    if (!/^(?:id|title|role|name|value|tabindex|type|class|aria-[\w-]+|data-[\w-]+)$/.test(name)) throw new Error("Unsupported component attribute: " + name);
    return value == null ? "" : ` ${name}="${escape(value)}"`;
  }).join("");
  const icon = (name) => root.GroveIcons?.(name) || "";
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
  function transcriptRow({ index, time, start, text, originalText, editing = false, disabled = false, speaker = "", speakerVisible = false, searchQuery = "" }) {
    const label = `${Number(index) + 1}번 발언`;
    const original = originalText != null && String(text ?? "") !== String(originalText)
      ? `<details class="transcript-original"><summary>원문 보기</summary><p>${escape(originalText)}</p></details>` : "";
    const content = editing
      ? `<textarea class="text-input"${attributes({ "data-segment": index, "aria-label": label + " 수정" })} rows="1" spellcheck="false"${disabled ? " readonly" : ""}>${escape(text)}</textarea><div class="transcript-edit-actions">${button({ text: "취소", className: "quiet small", disabled, attrs: { "data-transcript-cancel": index, "aria-label": label + " 수정 취소" } })}${button({ text: "완료", className: "small", disabled, attrs: { "data-transcript-done": index, "aria-label": label + " 수정 완료" } })}</div>`
      : `<p class="transcript-text">${highlightText(text, searchQuery)}</p>${original}`;
    return `<article${attributes({ class: "transcript-row" + (editing ? " is-editing" : "") + (speakerVisible ? " has-speaker" : ""), "data-transcript-row": index, tabindex: -1 })}>${button({ text: time, className: "transcript-time quiet small", attrs: { "data-seek": start, "aria-label": time + " 발언 재생" } })}${speakerVisible ? button({ html: highlightText(speaker || "화자 미지정", searchQuery), className: "transcript-speaker quiet small", disabled, attrs: { "data-speaker-edit": index, "aria-label": label + " 화자 변경: " + (speaker || "화자 미지정"), title: speaker || "화자 미지정" } }) : ""}<div class="transcript-content">${content}</div>${editing ? "" : `<div class="transcript-row-actions">${button({ text: "수정", className: "quiet small", disabled, attrs: { "data-transcript-edit": index, "aria-label": label + " 수정" } })}</div>`}</article>`;
  }
  function minutes(sections) {
    return `<article class="minutes-document">${sections.map(({ title, text, emptyText = "작성된 내용이 없습니다." }) => `<section><h3>${escape(title)}</h3><div class="minutes-text">${String(text || "").trim() ? escape(text) : `<span class="muted">${escape(emptyText)}</span>`}</div></section>`).join("")}</article>`;
  }
  function summaryDocument(value, { searchQuery = "" } = {}) {
    const headings = new Set(["회의 요약", "요약", "주요 내용", "논의 내용", "논의 사항", "결정 사항", "결정사항", "합의 사항", "다음 작업", "후속 작업", "할 일", "할일", "액션 아이템", "미결 사항", "확인 필요", "참고 사항"]);
    const term = String(searchQuery ?? "").trim();
    const pattern = term ? new RegExp(term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "giu") : null;
    let searchGroup = 0;
    const references = text => {
      const tokens = [];
      let cursor = 0;
      // Keep the saved one-based references. Never resolve them against current rows.
      for (const match of text.matchAll(/\[([0-9, \t-]+)\]/g)) {
        const before = text.slice(0, match.index), after = text.slice(match.index + match[0].length);
        if (/(?:\\|!)$/.test(before) || /^(?:\(|\[[^0-9])/.test(after)) continue;
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
        if (!valid || !refs.length) continue;
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
    const inline = text => {
      const tokens = [];
      let cursor = 0;
      for (const match of text.matchAll(/(\*\*|__)(.+?)\1|`([^`]+)`/g)) {
        tokens.push(...references(text.slice(cursor, match.index)));
        tokens.push(match[3] == null ? { tag: "strong", children: references(match[2]) } : { tag: "code", children: [{ text: match[3] }] });
        cursor = match.index + match[0].length;
      }
      tokens.push(...references(text.slice(cursor)));
      return renderTokens(tokens);
    };
    const blocks = [], paragraph = [];
    let list = null, code = null;
    const flushParagraph = () => { if (paragraph.length) blocks.push(`<p>${paragraph.splice(0).map(inline).join("<br>")}</p>`); };
    const flushList = () => { if (list) { blocks.push(`<${list.tag}${list.start == null ? "" : ` start="${list.start}"`}>${list.items.map(item => `<li>${item.map(inline).join("<br>")}</li>`).join("")}</${list.tag}>`); list = null; } };
    for (const line of String(value ?? "").replace(/\r\n?/g, "\n").split("\n")) {
      const trimmed = line.trim();
      if (/^```[^`]*$/.test(trimmed)) {
        flushParagraph(); flushList();
        if (code) { blocks.push(`<pre><code>${renderTokens([{ text: code.join("\n") }])}</code></pre>`); code = null; } else code = [];
        continue;
      }
      if (code) { code.push(line); continue; }
      if (!trimmed) { flushParagraph(); flushList(); continue; }
      const markdownHeading = trimmed.match(/^(#{1,6})\s+(.+?)(?:\s+#+)?$/);
      const plainHeading = trimmed.replace(/^(\*\*|__)(.+)\1$/, "$2").replace(/[:：]$/, "").trim();
      if (markdownHeading || headings.has(plainHeading)) {
        flushParagraph(); flushList();
        const level = markdownHeading ? Math.min(6, Math.max(3, markdownHeading[1].length + 1)) : 3;
        blocks.push(`<h${level}>${inline(markdownHeading ? markdownHeading[2] : plainHeading)}</h${level}>`);
        continue;
      }
      const item = line.match(/^\s*(?:([-*+])\s+|(\d+)[.)]\s+)(.+)$/);
      if (item) {
        flushParagraph();
        const tag = item[2] ? "ol" : "ul";
        if (list?.tag !== tag) { flushList(); list = { tag, items: [], start: tag === "ol" && Number.isSafeInteger(Number(item[2])) ? Number(item[2]) : null }; }
        list.items.push([item[3]]);
      } else if (list && /^\s{2,}\S/.test(line)) list.items.at(-1).push(trimmed);
      else { flushList(); paragraph.push(trimmed); }
    }
    flushParagraph(); flushList();
    if (code) blocks.push(`<pre><code>${renderTokens([{ text: code.join("\n") }])}</code></pre>`);
    return `<article class="summary-document">${blocks.join("")}</article>`;
  }
  function dialogContent({ title, titleId = "modal-title", bodyHTML = "", actionsHTML = "" }) {
    return `<h2${attributes({ id: titleId })}>${escape(title)}</h2>${bodyHTML}${actionsHTML ? `<div class="modal-footer"><span></span><div class="row">${actionsHTML}</div></div>` : ""}`;
  }
  const api = { escape, highlightText, button, badge, status, meetingRow, personRow, personOption, utterance, transcriptRow, minutes, summaryDocument, dialogContent };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.GroveUI = api;
})(typeof globalThis === "object" ? globalThis : this);
