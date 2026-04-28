#!/usr/bin/env node
function buildExtensionPayload(operation, payload) {
  if (operation === 'extension/event') {
    return {
      ok: true,
      handled_event: payload.event ?? 'unknown',
    };
  }
  if (operation === 'extension/command') {
    const commandName = payload.command_name ?? 'extension';
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

function emitResponse(line) {
  const trimmed = line.trim();
  if (!trimmed) {
    return;
  }
  const request = JSON.parse(trimmed);
  const method = request.method ?? '';
  const payload = request.payload ?? {};
  const responsePayload = method === 'tools/call'
    ? buildExtensionPayload(payload.operation ?? '', payload.payload ?? {})
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

process.stdin.on('data', (chunk) => {
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
