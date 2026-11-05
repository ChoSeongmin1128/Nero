// Messages form a tree: editing a question adds a sibling under the same parent. Older
// conversations have no parent ids and read as one line, matching the native store.
export const ROOT = 'root';

const parents = messages => messages.map((message, index) => message.parentId ?? (index ? messages[index - 1].id : ROOT));

export function conversationPath(conversation) {
  const messages = conversation?.messages || [], versions = conversation?.versions || {}, parent = parents(messages);
  const path = [];
  let key = ROOT;
  while (path.length < messages.length) {
    const children = messages.flatMap((_, index) => parent[index] === key ? [index] : []);
    if (!children.length) break;
    const chosen = children.find(index => messages[index].id === versions[key]) ?? children.at(-1);
    path.push(messages[chosen]);
    key = messages[chosen].id;
  }
  return path;
}

export function parentOf(conversation, messageId) {
  const messages = conversation?.messages || [], index = messages.findIndex(message => message.id === messageId);
  return index < 0 ? null : parents(messages)[index];
}

export function questionVersions(conversation, messageId) {
  const messages = conversation?.messages || [], parent = parents(messages), index = messages.findIndex(message => message.id === messageId);
  if (index < 0 || messages[index].role !== 'user') return { parent: null, siblings: [], position: -1 };
  const siblings = messages.filter((message, other) => message.role === 'user' && parent[other] === parent[index]);
  return { parent: parent[index], siblings, position: siblings.findIndex(message => message.id === messageId) };
}

// Questions sent in the shown conversation, newest last, for recalling with the arrow keys.
export function sentQuestions(conversation) {
  return conversationPath(conversation).filter(message => message.role === 'user' && message.text?.trim()).map(message => message.text);
}
