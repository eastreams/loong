#!/usr/bin/env node
type PayloadMap = Record<string, unknown>;

function buildExtensionPayload(operation: string, payload: PayloadMap): unknown {
  if (operation === 'extension/event') {
    const handledEvent = typeof payload.event === 'string' ? payload.event : 'unknown';
    const handledHook =
      typeof payload.host_hook === 'string' ? payload.host_hook : 'unknown';
    const handledTuiSurface =
      typeof payload.host_tui_surface === 'string' ? payload.host_tui_surface : 'unknown';
    return {
      ok: true,
      handled_event: handledEvent,
      handled_hook: handledHook,
      handled_tui_surface: handledTuiSurface,
      received_hook_payload:
        payload.hook_payload && typeof payload.hook_payload === 'object'
          ? payload.hook_payload
          : null,
      received_surface_payload:
        payload.surface_payload && typeof payload.surface_payload === 'object'
          ? payload.surface_payload
          : null,
    };
  }
  if (operation === 'extension/command') {
    const commandName =
      typeof payload.command_name === 'string' ? payload.command_name : 'extension';
    return {
      text: `${commandName} command stub`,
    };
  }
  if (operation === 'extension/resource') {
    return {
      commands: [],
      tools: [],
    };
  }
  return {
    error: `unsupported method: ${operation}`,
  };
}

function emitResponse(line: string): void {
  const trimmed = line.trim();
  if (!trimmed) {
    return;
  }
  const request = JSON.parse(trimmed) as {
    method?: string;
    id?: unknown;
    payload?: PayloadMap;
  };
  const method = typeof request.method === 'string' ? request.method : '';
  const payload = request.payload ?? {};
  const nestedPayload =
    payload.payload && typeof payload.payload === 'object'
      ? (payload.payload as PayloadMap)
      : {};
  const operation = typeof payload.operation === 'string' ? payload.operation : '';
  const responsePayload =
    method === 'tools/call'
      ? buildExtensionPayload(operation, nestedPayload)
      : { error: `unsupported transport method: ${method}` };
  const response = {
    method,
    id: request.id ?? null,
    payload: responsePayload,
  };
  process.stdout.write(`${JSON.stringify(response)}\n`);
}

process.stdin.setEncoding('utf8');
let buffered = '';

process.stdin.on('data', (chunk: string) => {
  buffered += chunk;
  let newlineIndex = buffered.indexOf('\n');
  while (newlineIndex !== -1) {
    const line = buffered.slice(0, newlineIndex);
    buffered = buffered.slice(newlineIndex + 1);
    emitResponse(line);
    newlineIndex = buffered.indexOf('\n');
  }
});

process.stdin.on('end', () => {
  if (buffered.trim()) {
    emitResponse(buffered);
  }
});

process.stdin.resume();
