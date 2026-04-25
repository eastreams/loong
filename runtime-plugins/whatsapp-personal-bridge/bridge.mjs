import http from 'node:http';
import { mkdir } from 'node:fs/promises';
import { parseArgs } from 'node:util';
import process from 'node:process';

import makeWASocket, {
  DisconnectReason,
  useMultiFileAuthState,
  fetchLatestBaileysVersion,
  makeCacheableSignalKeyStore,
  extractMessageContent as baileysExtractMessageContent,
} from '@whiskeysockets/baileys';
import { Boom } from '@hapi/boom';
import pino from 'pino';
import qrcode from 'qrcode-terminal';

const VERSION = '0.1.0';
const MAX_QUEUE_SIZE = 200;
const CONTACT_SUFFIX = '@s.whatsapp.net';
const GROUP_SUFFIX = '@g.us';

function parseCli() {
  const { values } = parseArgs({
    options: {
      host: { type: 'string', default: '127.0.0.1' },
      port: { type: 'string', default: '39731' },
      path: { type: 'string', default: '/bridge' },
      'auth-dir': { type: 'string' },
    },
    allowPositionals: false,
  });
  const port = Number.parseInt(values.port, 10);
  if (!Number.isFinite(port) || port <= 0) {
    throw new Error(`invalid --port ${values.port}`);
  }
  const authDir = values['auth-dir'];
  if (!authDir) {
    throw new Error('--auth-dir is required');
  }
  return {
    host: values.host,
    port,
    path: values.path.startsWith('/') ? values.path : `/${values.path}`,
    authDir,
  };
}

function normalizeRouteId(routeKind, rawRouteId) {
  const trimmed = `${rawRouteId ?? ''}`.trim();
  if (!trimmed) {
    throw new Error('target route id is empty');
  }
  if (routeKind === 'group') {
    return trimmed.includes('@') ? trimmed : `${trimmed}${GROUP_SUFFIX}`;
  }
  if (trimmed.includes('@')) {
    return trimmed;
  }
  const digits = trimmed.replace(/[^0-9]/g, '');
  if (!digits) {
    throw new Error(`contact target ${trimmed} is not a valid phone number or jid`);
  }
  return `${digits}${CONTACT_SUFFIX}`;
}

function parseCanonicalTarget(targetId) {
  const parts = `${targetId ?? ''}`.split(':');
  if (parts.length < 4) {
    throw new Error(`unexpected target id ${targetId}`);
  }
  const [channelId, configuredAccountId, routeKind, ...rest] = parts;
  if (channelId !== 'whatsapp-personal') {
    throw new Error(`unsupported channel ${channelId}`);
  }
  const routeId = rest.join(':');
  const jid = normalizeRouteId(routeKind, routeId);
  return { configuredAccountId, routeKind, routeId, jid };
}

function normalizeAllowedChatId(value) {
  const trimmed = `${value ?? ''}`.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed === '*') {
    return '*';
  }
  if (trimmed.includes('@')) {
    return trimmed;
  }
  const digits = trimmed.replace(/[^0-9]/g, '');
  if (!digits) {
    return trimmed;
  }
  return `${digits}${CONTACT_SUFFIX}`;
}

function readAllowedChatIds(runtimeContext) {
  const configured = runtimeContext?.account?.config?.allowed_chat_ids;
  if (!Array.isArray(configured)) {
    return [];
  }
  return configured
    .map(normalizeAllowedChatId)
    .filter((value) => typeof value === 'string' && value.length > 0);
}

function routeKindForJid(jid) {
  return `${jid ?? ''}`.endsWith(GROUP_SUFFIX) ? 'group' : 'contact';
}

function presentContactRouteId(jid) {
  const normalized = `${jid ?? ''}`.trim();
  if (normalized.endsWith(CONTACT_SUFFIX)) {
    return `+${normalized.slice(0, -CONTACT_SUFFIX.length)}`;
  }
  return normalized;
}

function outboundText(messagePayload) {
  if (typeof messagePayload?.Text === 'string' && messagePayload.Text.trim()) {
    return messagePayload.Text;
  }
  if (typeof messagePayload?.text === 'string' && messagePayload.text.trim()) {
    return messagePayload.text;
  }
  throw new Error('whatsapp-personal bridge only supports text outbound messages');
}

class WhatsAppBridge {
  constructor({ authDir }) {
    this.authDir = authDir;
    this.logger = pino({ level: 'silent' });
    this.sock = null;
    this.queue = [];
    this.recentMessageIds = new Set();
    this.connectionState = 'starting';
    this.lastQr = null;
    this.reconnecting = false;
  }

  async start() {
    await mkdir(this.authDir, { recursive: true });
    const { state, saveCreds } = await useMultiFileAuthState(this.authDir);
    const { version } = await fetchLatestBaileysVersion();

    this.sock = makeWASocket({
      auth: {
        creds: state.creds,
        keys: makeCacheableSignalKeyStore(state.keys, this.logger),
      },
      version,
      logger: this.logger,
      printQRInTerminal: false,
      browser: ['Loong', 'whatsapp-personal-bridge', VERSION],
      syncFullHistory: false,
      markOnlineOnConnect: false,
    });

    this.sock.ev.on('creds.update', saveCreds);
    this.sock.ev.on('connection.update', (update) => this.onConnectionUpdate(update));
    this.sock.ev.on('messages.upsert', (event) => this.onMessagesUpsert(event));
  }

  async stop() {
    if (this.sock?.end) {
      try {
        this.sock.end();
      } catch {
        // ignore
      }
    }
  }

  onConnectionUpdate(update) {
    const { connection, lastDisconnect, qr } = update;
    if (qr) {
      this.lastQr = qr;
      console.log('\n📱 Scan this QR code with WhatsApp Linked Devices:\n');
      qrcode.generate(qr, { small: true });
      console.log('\nIf the QR does not render well, resize the terminal and restart the bridge.\n');
    }

    if (connection === 'open') {
      this.connectionState = 'connected';
      console.log('✅ Connected to WhatsApp Personal bridge');
      return;
    }

    if (connection === 'close') {
      this.connectionState = 'disconnected';
      const statusCode = lastDisconnect?.error instanceof Boom
        ? lastDisconnect.error.output?.statusCode
        : undefined;
      const shouldReconnect = statusCode !== DisconnectReason.loggedOut;
      console.error(`WhatsApp connection closed (status=${statusCode ?? 'unknown'})`);
      if (shouldReconnect && !this.reconnecting) {
        this.reconnecting = true;
        setTimeout(async () => {
          this.reconnecting = false;
          try {
            await this.start();
          } catch (error) {
            console.error(`Reconnection failed: ${error}`);
          }
        }, 5000);
      }
    }
  }

  onMessagesUpsert(event) {
    if (event?.type !== 'notify' || !Array.isArray(event?.messages)) {
      return;
    }

    for (const message of event.messages) {
      const messageId = message?.key?.id;
      if (!messageId || this.recentMessageIds.has(messageId)) {
        continue;
      }
      if (message?.key?.fromMe || message?.key?.remoteJid === 'status@broadcast') {
        continue;
      }

      const remoteJid = `${message?.key?.remoteJid ?? ''}`.trim();
      if (!remoteJid) {
        continue;
      }

      const unwrapped = baileysExtractMessageContent(message.message);
      const text = this.extractText(unwrapped);
      if (!text) {
        continue;
      }

      this.recentMessageIds.add(messageId);
      if (this.recentMessageIds.size > MAX_QUEUE_SIZE * 4) {
        const first = this.recentMessageIds.values().next().value;
        if (first) this.recentMessageIds.delete(first);
      }

      this.queue.push({
        id: messageId,
        remoteJid,
        text,
        participantId: `${message?.key?.participant ?? ''}`.trim() || null,
      });
      if (this.queue.length > MAX_QUEUE_SIZE) {
        this.queue.shift();
      }
    }
  }

  extractText(unwrapped) {
    if (!unwrapped) return null;
    if (typeof unwrapped.conversation === 'string' && unwrapped.conversation.trim()) {
      return unwrapped.conversation;
    }
    if (typeof unwrapped.extendedTextMessage?.text === 'string' && unwrapped.extendedTextMessage.text.trim()) {
      return unwrapped.extendedTextMessage.text;
    }
    if (typeof unwrapped.imageMessage?.caption === 'string' && unwrapped.imageMessage.caption.trim()) {
      return `[image] ${unwrapped.imageMessage.caption}`;
    }
    if (typeof unwrapped.videoMessage?.caption === 'string' && unwrapped.videoMessage.caption.trim()) {
      return `[video] ${unwrapped.videoMessage.caption}`;
    }
    if (unwrapped.imageMessage) return '[image]';
    if (unwrapped.videoMessage) return '[video]';
    if (unwrapped.audioMessage) return '[audio]';
    if (unwrapped.documentMessage) return `[document] ${unwrapped.documentMessage.fileName ?? ''}`.trim();
    return null;
  }

  dequeue(runtimeContext) {
    const allowedChatIds = readAllowedChatIds(runtimeContext);
    const allowAll = allowedChatIds.length === 0 || allowedChatIds.includes('*');
    const remaining = [];
    const emitted = [];

    for (const item of this.queue) {
      const allowed = allowAll || allowedChatIds.includes(item.remoteJid);
      if (!allowed) {
        remaining.push(item);
        continue;
      }
      emitted.push(this.buildInboundMessage(item, runtimeContext));
    }

    this.queue = remaining;
    return emitted;
  }

  buildInboundMessage(item, runtimeContext) {
    const account = runtimeContext?.account ?? {};
    const configuredAccountId = account.configured_account_id ?? 'default';
    const accountId = account.account_id ?? configuredAccountId;
    const routeKind = routeKindForJid(item.remoteJid);
    const routeId = routeKind === 'contact' ? presentContactRouteId(item.remoteJid) : item.remoteJid;

    return {
      session: {
        platform: 'whatsapp',
        configured_account_id: configuredAccountId,
        account_id: accountId,
        conversation_id: item.remoteJid,
        participant_id: item.participantId,
        thread_id: null,
      },
      reply_target: {
        platform: 'whatsapp',
        kind: 'conversation',
        id: `whatsapp-personal:${configuredAccountId}:${routeKind}:${routeId}`,
        options: {},
      },
      text: item.text,
      delivery: {
        ack_cursor: item.id,
        source_message_id: item.id,
        sender_principal_key: item.participantId ?? item.remoteJid,
        thread_root_id: null,
        parent_message_id: null,
        resources: [],
        feishu_callback: null,
      },
    };
  }

  async send(targetId, messagePayload) {
    if (!this.sock) {
      throw new Error('WhatsApp bridge is not connected');
    }
    const { jid } = parseCanonicalTarget(targetId);
    const text = outboundText(messagePayload);
    await this.sock.sendMessage(jid, { text });
  }
}

function successResponse(body, payload) {
  return JSON.stringify({ method: body.method, id: body.id, payload });
}

async function main() {
  const cli = parseCli();
  const bridge = new WhatsAppBridge({ authDir: cli.authDir });
  await bridge.start();

  const server = http.createServer(async (req, res) => {
    if (req.method !== 'POST' || req.url !== cli.path) {
      res.writeHead(404, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ error: 'not_found' }));
      return;
    }

    let rawBody = '';
    req.setEncoding('utf8');
    req.on('data', (chunk) => {
      rawBody += chunk;
    });
    req.on('end', async () => {
      try {
        const body = JSON.parse(rawBody || '{}');
        const payload = body.payload ?? {};
        if (body.operation === 'send_message') {
          await bridge.send(payload?.target?.id, payload?.message ?? {});
          res.writeHead(200, { 'content-type': 'application/json' });
          res.end(successResponse(body, { ok: true }));
          return;
        }
        if (body.operation === 'receive_batch') {
          const messages = bridge.dequeue(payload?.runtime_context ?? {});
          res.writeHead(200, { 'content-type': 'application/json' });
          res.end(successResponse(body, { messages }));
          return;
        }
        if (body.operation === 'ack_inbound' || body.operation === 'complete_batch') {
          res.writeHead(200, { 'content-type': 'application/json' });
          res.end(successResponse(body, { ok: true }));
          return;
        }
        res.writeHead(400, { 'content-type': 'application/json' });
        res.end(successResponse(body, { error: `unsupported operation ${body.operation}` }));
      } catch (error) {
        res.writeHead(500, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ error: String(error) }));
      }
    });
  });

  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(cli.port, cli.host, () => {
      console.log(`🌉 WhatsApp Personal bridge listening on http://${cli.host}:${cli.port}${cli.path}`);
      resolve();
    });
  });

  const shutdown = async () => {
    server.close();
    await bridge.stop();
    process.exit(0);
  };
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
}

main().catch((error) => {
  console.error(`Bridge failed: ${error?.stack ?? error}`);
  process.exit(1);
});
