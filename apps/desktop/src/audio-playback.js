// A source replacement or an intentional pause can reject an outstanding play().
// Report only failures that still belong to the current playback request.
export async function requestAudioPlayback(audio, { isCurrent = () => true, onError = () => {} } = {}) {
  const source = audio.getAttribute('src');
  if (!source) return false;
  try {
    await audio.play();
    return isCurrent() && audio.getAttribute('src') === source;
  } catch (error) {
    if (error?.name !== 'AbortError' && isCurrent() && audio.getAttribute('src') === source) onError(error);
    return false;
  }
}
