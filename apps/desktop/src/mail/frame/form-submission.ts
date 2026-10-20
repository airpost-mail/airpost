export interface FormSubmissionTarget {
  url: string;
  method: 'get' | 'post';
}

/**
 * Where a form in a message would have gone: the action URL with the fields in its query
 * for GET, the bare action URL for POST. Null unless the action is an absolute http(s) URL.
 * Reads the attributes so a missing or relative action isn't resolved against about:blank.
 */
export function formSubmissionTarget(
  form: HTMLFormElement,
  submitter?: HTMLElement | null
): FormSubmissionTarget | null {
  if (!form || typeof form.getAttribute !== 'function') {
    return null;
  }
  const action = (submitter && submitter.getAttribute('formaction')) || form.getAttribute('action');
  const rawMethod =
    (submitter && submitter.getAttribute('formmethod')) || form.getAttribute('method');
  const method = (rawMethod || 'get').toLowerCase() === 'post' ? 'post' : 'get';

  let target: URL;
  try {
    target = new URL((action || '').trim());
  } catch {
    return null;
  }
  if (target.protocol !== 'https:' && target.protocol !== 'http:') {
    return null;
  }

  if (method === 'get') {
    const FrameFormData = (form.ownerDocument.defaultView as any)?.FormData || FormData;
    const data = new FrameFormData(form, submitter || undefined);
    const query = new URLSearchParams();
    for (const [key, value] of data.entries()) {
      if (typeof value === 'string') query.append(key, value);
    }
    target.search = query.toString();
  }
  return { url: target.toString(), method };
}
