/* global chrome */

const dispatch = async event => {
  // Only the app window's own requests; message frames post objects of their own.
  if (event.source !== window || typeof event.data !== 'string') return;
  let request;
  try {
    request = JSON.parse(event.data);
  } catch (err) {
    return;
  }
  const { call, text } = request || {};
  if (call === 'detectLanguage') {
    let result = null;
    let error = null;
    try {
      result = await chrome.i18n.detectLanguage(text);
    } catch (err) {
      error = err;
    }
    window.postMessage(JSON.stringify({ response: 'detectLanguage', text, result, error }), '*');
  }
};

window.addEventListener('message', dispatch, false);
