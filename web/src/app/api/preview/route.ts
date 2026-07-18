import { proxyToRust, TIMEOUTS } from '@/lib/api-config';

export async function POST(request: Request): Promise<Response> {
  let formData: FormData;
  try {
    formData = await request.formData();
  } catch {
    return Response.json({ error: 'Failed to parse form data' }, { status: 400 });
  }

  const file = formData.get('file');
  if (!file || !(file instanceof File)) {
    return Response.json({ error: 'No file field found in form data' }, { status: 400 });
  }

  // Forward multipart to Rust API /preview
  const forwardForm = new FormData();
  forwardForm.append('file', file);

  // Forward any additional form fields (e.g. options)
  for (const [key, value] of formData.entries()) {
    if (key !== 'file') {
      forwardForm.append(key, value);
    }
  }

  return proxyToRust('/preview', {
    method: 'POST',
    body: forwardForm,
    timeoutMs: TIMEOUTS.CONVERT,
    timeoutLabel: 'Preview',
  });
}
