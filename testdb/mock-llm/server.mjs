// OpenAI-compatible mock LLM for sandb tests. Zero dependencies.
// The scenario is picked by a keyword in the last user message; see README.md.
import http from 'node:http';

const HOST = process.env.HOST || '127.0.0.1';
const PORT = Number(process.env.PORT || 58080);
const KEY = process.env.MOCK_LLM_KEY || '';
const MODEL = 'mock-gpt';

const toolScenarios = {
  list: ['list_tables', {}],
  describe: ['describe_table', { table: 'orders' }],
  count: ['run_read_query', { sql: 'SELECT count(*) FROM orders' }],
  bypass: ['run_read_query', { sql: 'DELETE FROM orders' }],
  write: ['propose_write', { sql: "UPDATE orders SET status = 'shipped' WHERE id = 1" }],
};
// Checked in this order, so "error" and "slow" win over tool keywords.
const keywords = ['error', 'slow', 'bypass', 'write', 'describe', 'count', 'list'];

let seq = 0;
const nextId = (p) => `${p}_${Date.now().toString(36)}${(++seq).toString(36)}`;

// Parsed bodies of the last POST /v1/chat/completions requests, oldest first, for GET /_requests.
const MAX_RECORDED = 100;
const recorded = [];

const textOf = (content) =>
  typeof content === 'string'
    ? content
    : Array.isArray(content)
      ? content.map((p) => (typeof p === 'string' ? p : p?.text ?? '')).join('')
      : '';

function plan(body) {
  const msgs = Array.isArray(body.messages) ? body.messages : [];
  const last = msgs[msgs.length - 1];
  if (last?.role === 'tool') {
    return { kind: 'text', text: `Tool result received: ${textOf(last.content).slice(0, 200)}` };
  }
  const lastUser = [...msgs].reverse().find((m) => m.role === 'user');
  const q = textOf(lastUser?.content).toLowerCase();
  const kw = keywords.find((k) => new RegExp(`\\b${k}\\b`).test(q));
  if (kw === 'error') return { kind: 'error' };
  if (kw === 'slow') return { kind: 'slow' };
  if (kw) {
    const [name, args] = toolScenarios[kw];
    return { kind: 'tool', name, args: JSON.stringify(args) };
  }
  return { kind: 'text', text: `Echo: ${textOf(lastUser?.content)}` };
}

const send = (res, status, obj) => {
  res.writeHead(status, { 'content-type': 'application/json' });
  res.end(JSON.stringify(obj));
};

function chunk(id, delta, finish = null) {
  return `data: ${JSON.stringify({
    id, object: 'chat.completion.chunk', created: Math.floor(Date.now() / 1000), model: MODEL,
    choices: [{ index: 0, delta, finish_reason: finish }],
  })}\n\n`;
}

const pieces = (s, n) => {
  const size = Math.max(1, Math.ceil(s.length / n));
  const out = [];
  for (let i = 0; i < s.length; i += size) out.push(s.slice(i, i + size));
  return out.length ? out : [''];
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function stream(req, res, p) {
  const id = nextId('chatcmpl');
  let closed = false;
  res.on('close', () => { closed = true; });
  res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' });
  res.write(chunk(id, { role: 'assistant', content: '' }));

  if (p.kind === 'tool') {
    const callId = nextId('call');
    res.write(chunk(id, { tool_calls: [{ index: 0, id: callId, type: 'function', function: { name: p.name, arguments: '' } }] }));
    for (const frag of pieces(p.args, 3)) res.write(chunk(id, { tool_calls: [{ index: 0, function: { arguments: frag } }] }));
    res.write(chunk(id, {}, 'tool_calls'));
  } else if (p.kind === 'slow') {
    for (let i = 1; i <= 20; i++) {
      if (closed) { console.log(`[${id}] client closed during slow stream at chunk ${i}`); return; }
      res.write(chunk(id, { content: `chunk ${i} ` }));
      await sleep(300);
    }
    res.write(chunk(id, {}, 'stop'));
  } else {
    for (const frag of pieces(p.text, 4)) res.write(chunk(id, { content: frag }));
    res.write(chunk(id, {}, 'stop'));
  }
  res.write('data: [DONE]\n\n');
  res.end();
}

function complete(res, p) {
  const message =
    p.kind === 'tool'
      ? { role: 'assistant', content: null, tool_calls: [{ id: nextId('call'), type: 'function', function: { name: p.name, arguments: p.args } }] }
      : { role: 'assistant', content: p.kind === 'slow' ? Array.from({ length: 20 }, (_, i) => `chunk ${i + 1} `).join('') : p.text };
  send(res, 200, {
    id: nextId('chatcmpl'), object: 'chat.completion', created: Math.floor(Date.now() / 1000), model: MODEL,
    choices: [{ index: 0, message, finish_reason: p.kind === 'tool' ? 'tool_calls' : 'stop' }],
    usage: { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 },
  });
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://x');
  if (req.method === 'GET' && url.pathname === '/health') return send(res, 200, { ok: true });

  if (KEY && req.headers.authorization !== `Bearer ${KEY}`) {
    return send(res, 401, { error: { message: 'invalid api key', type: 'invalid_request_error', code: 'invalid_api_key' } });
  }
  if (req.method === 'GET' && url.pathname === '/v1/models') {
    return send(res, 200, { object: 'list', data: [{ id: MODEL, object: 'model', created: 0, owned_by: 'sandb-test' }] });
  }
  if (req.method === 'GET' && url.pathname === '/_requests') return send(res, 200, recorded);
  if (req.method !== 'POST' || url.pathname !== '/v1/chat/completions') {
    return send(res, 404, { error: { message: `no route ${req.method} ${url.pathname}`, type: 'not_found' } });
  }

  let raw = '';
  for await (const c of req) raw += c;
  console.log(`${new Date().toISOString()} POST /v1/chat/completions ${raw.slice(0, 500)}${raw.length > 500 ? '...' : ''}`);
  let body;
  try {
    body = JSON.parse(raw);
  } catch {
    return send(res, 400, { error: { message: 'invalid JSON body', type: 'invalid_request_error' } });
  }
  recorded.push(body);
  if (recorded.length > MAX_RECORDED) recorded.shift();

  const p = plan(body);
  if (p.kind === 'error') {
    return send(res, 500, { error: { message: 'mock server error (scenario: error)', type: 'server_error', code: 'mock_error' } });
  }
  if (body.stream) return stream(req, res, p);
  return complete(res, p);
});

server.listen(PORT, HOST, () => console.log(`mock-llm listening on http://${HOST}:${PORT}${KEY ? ' (bearer key required)' : ''}`));
for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, () => server.close(() => process.exit(0)));
