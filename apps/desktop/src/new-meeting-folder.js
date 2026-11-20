// Where new meetings are filed. Home and the recording settings show and change the same value.
export function createNewMeetingFolder({ bridge, onChange = () => {} }) {
  let value = null, busy = false;
  return {
    value: () => value,
    busy: () => busy,
    async reload() {
      try { value = await bridge.readNewMeetingFolder() ?? null; } catch { return; }
      onChange();
    },
    // A failed change keeps the previous folder and rethrows for the screen that asked.
    async set(folderId) {
      const previous = value;
      busy = true; onChange();
      try { value = await bridge.setNewMeetingFolder(folderId || null) ?? null; }
      catch (error) { value = previous; throw error; }
      finally { busy = false; onChange(); }
    },
  };
}
