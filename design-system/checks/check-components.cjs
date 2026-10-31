const assert = require('node:assert/strict');
const UI = require('../components.js');
const { test } = require('node:test');

test('component text and attribute values cannot inject markup', () => {
  const value = '\"><img src=x onerror=alert(1)>';
  const button = UI.button({ text: value, action: value, attrs: { 'aria-label': value } });
  assert.ok(!button.includes('<img'));
  assert.ok(button.includes('&lt;img'));
  assert.throws(() => UI.button({ text: '등록', attrs: { onclick: 'bad()' } }), /Unsupported/);
  const row = UI.meetingRow({ title: value, date: value, phase: 'recording', count: 36, label: '녹음 중' });
  assert.ok(!row.includes('<img'));
  assert.match(row, /title="&quot;&gt;/);
  assert.ok(row.includes('36명'));
});

test('shared rows preserve event ownership and semantic state', () => {
  const row = UI.meetingRow({ title: '회의', date: '날짜 미정', phase: 'paused', label: '일시정지', action: 'open-meeting:42' });
  assert.ok(row.includes('data-action="open-meeting:42"'));
  assert.ok(row.includes('status-chip paused'));
  assert.ok(!row.includes('with-icon'), 'A structured row must keep its grid layout instead of the icon-button layout');
  const option = UI.personOption({ name: '홍길동', checked: true, attrs: { 'data-field': 'attendee:42', value: 42 } });
  assert.ok(option.includes('data-field="attendee:42"'));
  assert.ok(option.includes(' checked'));
  assert.ok(!UI.personOption({ name: '홍길동' }).includes(' checked'));
});

test('transcript distinguishes unknown speaker, current turn and edits', () => {
  const current = UI.utterance({ speaker: '발언자 1', time: '00:42', timeAction: 'time:42', unknown: true, current: true, text: '<발언>' });
  assert.ok(current.includes('이름 미확인'));
  assert.ok(current.includes('data-action="time:42"'));
  assert.ok(current.includes('aria-label="00:42 발언"'));
  assert.ok(current.includes('class="pending"'));
  assert.ok(current.includes('&lt;발언&gt;'));
  const edit = UI.utterance({ speaker: '홍길동', time: '00:42', editing: true, contentHTML: '<textarea id="edit-text"></textarea>' });
  assert.ok(edit.includes('id="edit-text"'));
  assert.ok(!edit.includes('class="pending"'));
});

test('transcript reading rows preserve time and edits without inventing speaker identity', () => {
  const row = UI.transcriptRow({ index: 0, time: '00:42', start: 42, text: '회의 내용입니다.', originalText: '회의 내용입니다.' });
  assert.match(row, /<article class="transcript-row" data-transcript-row="0" tabindex="-1">/);
  assert.match(row, /data-seek="42" aria-label="00:42 발언 재생"/);
  assert.match(row, /<p class="transcript-text">회의 내용입니다\.<\/p>/);
  assert.match(row, /data-transcript-edit="0" aria-label="1번 발언 수정"/);
  assert.doesNotMatch(row, /textarea|<details|speaker|avatar/);
  const corrected = UI.transcriptRow({ index: 1, time: '01:05', start: 65, text: '<수정된 내용>', originalText: '<이전 내용>' });
  assert.match(corrected, /<p class="transcript-text">&lt;수정된 내용&gt;<\/p>/);
  assert.doesNotMatch(corrected, /원문 보기|transcript-original|<details|이전 내용/);
  assert.ok(!corrected.includes('<수정된 내용>'));
  assert.ok(!corrected.includes('<이전 내용>'));
});

test('transcript editor exposes a single compact field with explicit completion and cancellation', () => {
  const row = UI.transcriptRow({ index: 2, time: '01:18', start: 78, text: '</textarea><script>bad()</script>', originalText: '원문', editing: true, disabled: true });
  assert.match(row, /class="transcript-row is-editing"/);
  assert.match(row, /<textarea class="text-input" data-segment="2" aria-label="3번 발언 수정" rows="1" spellcheck="false" readonly>/);
  assert.match(row, /&lt;\/textarea&gt;&lt;script&gt;bad\(\)&lt;\/script&gt;/);
  assert.equal((row.match(/<textarea\b/g) || []).length, 1);
  assert.match(row, /data-transcript-done="2"[^>]* disabled>완료<\/button>/);
  assert.match(row, /data-transcript-cancel="2"[^>]* disabled>취소<\/button>/);
  assert.doesNotMatch(row, /data-transcript-edit=|<details|<script>/);
});

test('document and dialog render safe content without owning app state', () => {
  const minutes = UI.minutes([{ title: '회의 내용', text: '' }, { title: '할 일', text: '<script>\n두 번째 줄' }]);
  assert.ok(minutes.includes('작성된 내용이 없습니다.'));
  assert.ok(minutes.includes('&lt;script&gt;\n두 번째 줄'));
  assert.ok(!minutes.includes('<script>'));
  const modal = UI.dialogContent({ title: '<긴 제목>', titleId: 'meeting-dialog', actionsHTML: UI.button({ text: '취소', action: 'close' }) });
  assert.ok(modal.includes('id="meeting-dialog"'));
  assert.ok(modal.includes('&lt;긴 제목&gt;'));
  assert.ok(modal.includes('data-action="close"'));
});

test('literal search highlighting escapes text and treats query metacharacters as text', () => {
  const html = UI.highlightText('<img>A+B a+b AaB</img>', 'a+b');
  assert.equal((html.match(/<mark class="search-match">/g) || []).length, 2);
  assert.match(html, /&lt;img&gt;<mark class="search-match">A\+B<\/mark>/);
  assert.match(html, / AaB&lt;\/img&gt;$/);
  assert.equal(UI.highlightText('<script>"&', '  '), '&lt;script&gt;&quot;&amp;');
  assert.equal(UI.highlightText('a.b aXb', '.'), 'a<mark class="search-match">.</mark>b aXb');
  assert.equal(UI.highlightText('ABC한글abc', 'abc'), '<mark class="search-match">ABC</mark>한글<mark class="search-match">abc</mark>');
  assert.doesNotMatch(UI.highlightText('<img src=x onerror=bad()>', '<img'), /<img|onerror="/);
});

test('summary structures known headings, markdown headings, paragraphs and lists', () => {
  const html = UI.summaryDocument('결정 사항\n기존 계획을 유지합니다.\n\n## 다음 작업\n- **확인** 내용을 공유합니다.\n  자세한 내용입니다.\n- 다른 작업\n\n3. 세 번째\n4. 네 번째\n\n결정 사항을 다시 확인합니다.');
  assert.match(html, /^<article class="summary-document"><h3>결정 사항<\/h3><p>기존 계획을 유지합니다\.<\/p>/);
  assert.match(html, /<h3>다음 작업<\/h3><ul><li><strong>확인<\/strong> 내용을 공유합니다\.<br>자세한 내용입니다\.<\/li><li>다른 작업<\/li><\/ul>/);
  assert.match(html, /<ol start="3"><li>세 번째<\/li><li>네 번째<\/li><\/ol>/);
  assert.match(html, /<p>결정 사항을 다시 확인합니다\.<\/p>/);
  assert.match(UI.summaryDocument('**할 일:**\n내용'), /<h3>할 일<\/h3>/);
  assert.equal(UI.summaryDocument(''), '<article class="summary-document"></article>');
});

test('summary evidence preserves frozen references without consulting current source positions', () => {
  const html = UI.summaryDocument('# <script>bad()</script>\n내용 [1] [0] [2] [999999999999999999999999]\n\n[x](javascript:bad())', { sources: ['"><img src=x onerror=bad()>'] });
  assert.doesNotMatch(html, /<script|<img|<a\b|data-summary-seek/);
  assert.deepEqual([...html.matchAll(/title="([^"]*)"/g)].map(match => match[1]), ['음성 기록에서 발언 확인', '음성 기록에서 발언 확인']);
  assert.match(html, /<h3>&lt;script&gt;bad\(\)&lt;\/script&gt;<\/h3>/);
  assert.equal((html.match(/data-summary-citations=/g) || []).length, 2);
  assert.match(html, /data-summary-citations="1"/);
  assert.match(html, /data-summary-citations="2"/);
  assert.match(html, /\[0\]/);
  assert.match(html, /\[999999999999999999999999\]/);
  assert.match(UI.summaryDocument('내용 [1]', { sources: [] }), /data-summary-citations="1"/);
  const code = UI.summaryDocument('```html\n<script> [1]\n# 제목\n```');
  assert.match(code, /<pre><code>&lt;script&gt; \[1\]\n# 제목<\/code><\/pre>/);
  assert.doesNotMatch(code, /<button|<h3>/);
});

test('summary groups adjacent comma and bounded range citations without altering identifiers', () => {
  const html = UI.summaryDocument('내용 [1] [2, 3] [3-5] 뒤 [7]');
  assert.equal((html.match(/<button/g) || []).length, 2);
  assert.match(html, /data-summary-citations="1,2,3,4,5"/);
  assert.match(html, /aria-label="1, 2, 3, 4, 5번 발언 확인"/);
  assert.match(html, /aria-pressed="false"/);
  assert.doesNotMatch(html, /aria-haspopup|aria-expanded/);
  assert.match(html, /data-summary-citations="7"/);
  for (const input of ['[0]', '[5-3]', '[1-33]', '[1-99999999999]', '[1,,2]', '[9007199254740992]']) assert.doesNotMatch(UI.summaryDocument(input), /<button/);
  const adjacent = UI.summaryDocument('[1-32][33]');
  assert.equal((adjacent.match(/<button/g) || []).length, 2);
  assert.doesNotMatch(UI.summaryDocument('`[1]` \\[2] [3](url) ![4](url) [5][name]'), /<button/);
});

test('search highlighting leaves summary source controls and editable transcript fields intact', () => {
  const summary = UI.summaryDocument('요약\nAlpha [1] alpha', { sources: ['원문 Alpha'], searchQuery: 'alpha' });
  assert.equal((summary.match(/<mark class="search-match" /g) || []).length, 2);
  assert.match(summary, /data-summary-citations="1"[^>]*aria-label="1번 발언 확인"/);
  assert.match(UI.summaryDocument('내용 [1]', { sources: ['원문'], searchQuery: '1' }), /data-search-group="summary-1">근거<\/mark>/);
  const row = UI.transcriptRow({ index: 0, time: '00:00', start: 0, text: 'Alpha <value>', searchQuery: 'alpha' });
  assert.match(row, /<p class="transcript-text"><mark class="search-match">Alpha<\/mark> &lt;value&gt;<\/p>/);
  const editor = UI.transcriptRow({ index: 0, time: '00:00', start: 0, text: 'Alpha <value>', searchQuery: 'alpha', editing: true });
  assert.doesNotMatch(editor, /<mark/);
  assert.match(editor, />Alpha &lt;value&gt;<\/textarea>/);
  const named = UI.transcriptRow({ index: 0, time: '00:00', start: 0, text: '내용', speaker: '<Alpha> "name"', speakerVisible: true, searchQuery: 'alpha' });
  assert.match(named, /aria-label="1번 발언 화자 변경: &lt;Alpha&gt; &quot;name&quot;" title="&lt;Alpha&gt; &quot;name&quot;">&lt;<mark class="search-match">Alpha<\/mark>&gt; &quot;name&quot;<\/button>/);
  assert.doesNotMatch(named, /<Alpha>/);
});

test('summary searches visible text across strong, code and reference boundaries as one result', () => {
  const text = '**계획** `일정` 확인 [1] / **계획** `일정` 확인 [1]';
  const options = { sources: ['원문 <내용> "확인"'] };
  const html = UI.summaryDocument(text, { ...options, searchQuery: '계획 일정 확인 [1]' });
  const marks = [...html.matchAll(/<mark class="search-match" data-search-group="([^"]+)">([^<]+)<\/mark>/g)];
  assert.deepEqual(marks.map(mark => mark[2]), ['계획', ' ', '일정', ' 확인 ', '근거', '[1]', '계획', ' ', '일정', ' 확인 ', '근거', '[1]']);
  assert.deepEqual(marks.map(mark => mark[1]), [...Array(6).fill('summary-1'), ...Array(6).fill('summary-2')]);
  assert.equal(html.replace(/<mark\b[^>]*>|<\/mark>/g, ''), UI.summaryDocument(text, options));
  assert.equal((html.match(/data-summary-citations="1"/g) || []).length, 2);
  assert.match(html, /aria-label="1번 발언 확인"><span aria-hidden="true">[\s\S]*?<\/span><span class="sr-only"><mark/);
  assert.doesNotMatch(html, /<mark\b[^>]*><\/mark>/);
  assert.doesNotMatch(UI.summaryDocument('**계획** `일정`', { searchQuery: '**' }), /<mark/);
});

test('summary searches reference labels without changing their click or escape contract', () => {
  const text = '**[1]** 뒤 [0] [2]';
  const options = { sources: ['"><img src=x onerror=bad()>'] };
  const html = UI.summaryDocument(text, { ...options, searchQuery: '[1] 뒤' });
  assert.match(html, /<strong><button[^>]*data-summary-citations="1"[^>]*aria-label="1번 발언 확인"><span aria-hidden="true">[\s\S]*?<\/span><span class="sr-only"><mark class="search-match" data-search-group="summary-1">근거<\/mark>/);
  assert.equal(html.replace(/<mark\b[^>]*>|<\/mark>/g, ''), UI.summaryDocument(text, options));
  assert.doesNotMatch(html, /<img|<script|<a\b|<mark\b[^>]*><\/mark>/);
  const invalid = UI.summaryDocument(text, { ...options, searchQuery: '[2]' });
  assert.match(invalid, /<mark class="search-match" data-search-group="summary-1">\[2\]<\/mark>/);
  assert.equal((invalid.match(/<button\b/g) || []).length, 2);
});

test('summary searches fenced code literally with and without a closing fence', () => {
  for (const suffix of ['\n```', '']) {
    const text = '```html\n<script> 계획 [1]\n계획 & "확인"' + suffix;
    const options = { sources: ['원문'] };
    const html = UI.summaryDocument(text, { ...options, searchQuery: '계획' });
    assert.equal((html.match(/<mark class="search-match" /g) || []).length, 2);
    assert.match(html, /&lt;script&gt; <mark class="search-match" data-search-group="summary-1">계획<\/mark> \[1\]\n<mark class="search-match" data-search-group="summary-2">계획<\/mark> &amp; &quot;확인&quot;/);
    assert.equal(html.replace(/<mark\b[^>]*>|<\/mark>/g, ''), UI.summaryDocument(text, options));
    assert.doesNotMatch(html, /<button|<script|<h3>/);
  }
});

test('summary match groups stay unique across blocks and preserve literal Unicode content', () => {
  const text = '## <값>\n**<값>**\n\n- `<값>`\n\n```\n<값>\n```';
  const html = UI.summaryDocument(text, { searchQuery: '<값>' });
  assert.deepEqual([...html.matchAll(/data-search-group="([^"]+)"/g)].map(match => match[1]), ['summary-1', 'summary-2', 'summary-3', 'summary-4']);
  assert.equal(html.replace(/<mark\b[^>]*>|<\/mark>/g, ''), UI.summaryDocument(text));
  assert.doesNotMatch(html, /<값>/);
  const unicode = UI.summaryDocument('앞 **가**`나` A+B a+b', { searchQuery: '가나' });
  assert.match(unicode, /<strong><mark class="search-match" data-search-group="summary-1">가<\/mark><\/strong><code><mark class="search-match" data-search-group="summary-1">나<\/mark><\/code>/);
  assert.equal((UI.summaryDocument('**A+**`B` a+b', { searchQuery: 'a+b' }).match(/data-search-group="summary-2"/g) || []).length, 1);
});
