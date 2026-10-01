(function installDesktopRuntime(metadata, origin) {
  if (location.origin !== origin) return;
  Object.defineProperty(window, '__COGNUUM_DESKTOP__', {
    value: Object.freeze(metadata), configurable: false,
  });

  // WKWebView may expose Web Speech even when its recognition service cannot
  // run in this app. Max owns microphone capture through the native bridge;
  // feature detection must not offer a second, non-working browser mic.
  for (const name of ['SpeechRecognition', 'webkitSpeechRecognition']) {
    Object.defineProperty(window, name, { value: undefined, writable: false, configurable: false });
  }

  const mark = () => {
    document.documentElement.dataset.tauri = 'true';
    document.documentElement.dataset.appVersion = window.__COGNUUM_DESKTOP__.version;
  };
  if (document.documentElement) mark();
  else document.addEventListener('DOMContentLoaded', mark, { once: true });
})
