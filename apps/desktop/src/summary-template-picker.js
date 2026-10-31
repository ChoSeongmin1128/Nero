export function mountSummaryTemplatePicker({ node, bridge, storage = localStorage, onAvailabilityChange = () => {} }) {
  let revision = 0, entries = [], selection = 'meeting', loading = false;
  const isAvailable = () => !loading && entries.some(item => item.id === selection);
  const notify = () => onAvailabilityChange(isAvailable());
  try { selection = storage.getItem('nero-summary-template') || selection; } catch {}
  async function reload() {
    const request = ++revision;
    loading = true; node.disabled = true; notify();
    try {
      const templates = await bridge.listTemplates();
      if (request !== revision) return;
      entries = templates;
      if (!entries.some(item => item.id === selection)) selection = '';
      const options = entries.map(item => {
        const option = node.ownerDocument.createElement('option');
        option.value = item.id; option.textContent = item.name; option.selected = item.id === selection;
        return option;
      });
      if (!selection) {
        const option = node.ownerDocument.createElement('option');
        option.value = ''; option.textContent = entries.length ? '템플릿을 다시 선택하세요' : '템플릿 없음';
        option.disabled = true; option.selected = true; options.unshift(option);
      }
      node.replaceChildren(...options);
      node.disabled = !entries.length;
      node.title = entries.length ? '' : '설정에서 템플릿을 추가하거나 전체 초기화로 기본 템플릿을 복원해 주세요.';
      loading = false; notify();
    } catch {
      if (request !== revision) return;
      entries = []; loading = false; node.disabled = true;
      node.title = '템플릿을 불러오지 못했습니다. 설정의 요약 템플릿에서 확인해 주세요.'; notify();
    }
  }
  node.addEventListener('change', () => { selection = node.value; try { storage.setItem('nero-summary-template', selection); } catch {} notify(); });
  return { reload, isAvailable, async selected() {
    if (loading || !entries.length) await reload();
    const entry = entries.find(item => item.id === selection);
    if (!entry) throw Error(entries.length ? '요약 템플릿을 다시 선택해 주세요.' : '요약 템플릿이 없습니다. 설정에서 추가하거나 전체 초기화해 주세요.');
    try {
      const current = await bridge.getTemplate(entry.id);
      return { id: current.id, revision: current.revision };
    } catch (error) {
      await reload();
      throw Error(error?.message || '선택한 템플릿을 확인하지 못했습니다. 템플릿을 다시 선택해 주세요.');
    }
  } };
}
