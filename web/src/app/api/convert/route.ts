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

  const optionsRaw = formData.get('options');
  const optionsStr = typeof optionsRaw === 'string' ? optionsRaw : '{}';

  // Forward multipart to Rust API /convert
  const forwardForm = new FormData();
  forwardForm.append('file', file);
  forwardForm.append('options', optionsStr);

  return proxyToRust('/convert', {
    method: 'POST',
    body: forwardForm,
    timeoutMs: TIMEOUTS.CONVERT,
    timeoutLabel: 'Convert',
  });
}
