export function mountSummaryTemplatePicker({ node, bridge, storage = localStorage }) {
  let revision = 0, entries = [], selection = 'meeting';
  try { selection = storage.getItem('nero-summary-template') || selection; } catch {}
  async function reload() {
    const request = ++revision;
    node.disabled = true;
    try {
      const templates = await bridge.listTemplates();
      if (request !== revision) return;
      entries = templates;
      if (!entries.some(item => item.id === selection)) selection = entries[0]?.id || '';
      node.replaceChildren(...entries.map(item => {
        const option = node.ownerDocument.createElement('option');
        option.value = item.id; option.textContent = item.name; option.selected = item.id === selection;
        return option;
      }));
      node.disabled = !entries.length;
      node.title = '';
    } catch {
      if (request !== revision) return;
      entries = []; node.disabled = true; node.title = '템플릿을 불러오지 못했습니다. 요약을 누르면 다시 확인합니다.';
    }
  }
  node.addEventListener('change', () => { selection = node.value; try { storage.setItem('nero-summary-template', selection); } catch {} });
  return { reload, async selected() {
    if (!entries.length) await reload();
    const entry = entries.find(item => item.id === selection);
    if (!entry) throw Error('요약 템플릿을 불러오지 못했습니다. 설정에서 확인해 주세요.');
    try {
      const current = await bridge.getTemplate(entry.id);
      return { id: current.id, revision: current.revision };
    } catch (error) {
      await reload();
      throw Error(error?.message || '선택한 템플릿을 확인하지 못했습니다. 템플릿을 다시 선택해 주세요.');
    }
  } };
}
