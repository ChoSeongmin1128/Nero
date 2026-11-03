import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

function call(command, args = {}) {
  if (!isTauri()) return Promise.reject(new Error('Nero 앱에서 열어 주세요.'));
  return invoke(command, args).catch(error => { throw new Error(typeof error === 'string' ? error : '요청을 처리하지 못했습니다. 다시 시도해 주세요.'); });
}
export const composerCatalog = () => call('ai_chat_composer_catalog');
export const accounts = () => call('ai_chat_accounts');
export const connect = (provider, accountId = null) => call('ai_chat_connect', { provider, accountId });
export const finishConnect = attemptId => call('ai_chat_finish_connect', { attemptId });
export const cancelConnect = attemptId => call('ai_chat_cancel_connect', { attemptId });
export const disconnect = accountId => call('ai_chat_disconnect', { accountId });
export const models = (accountId, refresh = true) => call('ai_chat_models', { accountId, refresh });
export const list = () => call('ai_chat_list');
export const create = () => call('ai_chat_create');
export const read = conversationId => call('ai_chat_read', { conversationId });
export const remove = conversationId => call('ai_chat_delete', { conversationId });
export const send = request => call('ai_chat_send', request);
export const cancel = requestId => call('ai_chat_cancel', { requestId });
export const saveSummary = request => call('ai_chat_save_summary', request);
export const appendNote = request => call('ai_chat_append_note', request);
export const subscribe = callback => isTauri() ? listen('nero://ai-chat', event => callback(event.payload)) : Promise.resolve(() => {});
export const notionSnapshot = () => call('notion_connection_snapshot');
export const notionConnect = () => call('begin_notion_connect');
export const notionFinish = attemptId => call('finish_notion_connect', { attemptId });
export const notionCancel = attemptId => call('cancel_notion_connect', { attemptId });
export const notionDisconnect = () => call('disconnect_notion_context');
export const fetchSources = request => call('fetch_notion_sources', request);
export const cancelFetch = requestId => call('cancel_notion_fetch', { requestId });
export const openSource = url => call('ai_chat_open_source', { url });
export const exportAnswer = request => call('ai_chat_answer_document', request);

export const meetingSources = meetingId => call('meeting_context_sources', { meetingId });
export const meetingAddSource = request => call('meeting_context_add', request);
export const meetingRemoveSource = request => call('meeting_context_remove', request);
export const meetingRefreshSource = request => call('meeting_context_refresh', request);
