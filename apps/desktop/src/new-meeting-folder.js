// Where new meetings are filed. Home and the recording settings show and change the same value.
export function createNewMeetingFolder({ bridge, onChange = () => {} }) {
  let value = null, busy = false, version = 0;
  return {
    value: () => value,
    busy: () => busy,
    // A read that started before a change, or finished while one runs, would bring back the old folder.
    async reload() {
      const started = version;
      let read;
      try { read = await bridge.readNewMeetingFolder() ?? null; } catch { return; }
      if (started !== version || busy) return;
      value = read;
      onChange();
    },
    // A failed change keeps the previous folder and rethrows for the screen that asked.
    async set(folderId) {
      const previous = value;
      version += 1;
      busy = true; onChange();
      try { value = await bridge.setNewMeetingFolder(folderId || null) ?? null; }
      catch (error) { value = previous; throw error; }
      finally { busy = false; onChange(); }
    },
  };
}
