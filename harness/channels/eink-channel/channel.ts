#!/usr/bin/env -S node --experimental-strip-types
import { createServer } from 'node:http'
import { Server } from '@modelcontextprotocol/sdk/server/index.js'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { ListToolsRequestSchema, CallToolRequestSchema } from '@modelcontextprotocol/sdk/types.js'

// Port 0 means "any free port": two Claude instances must be able to coexist, and a
// fixed port makes the second one fail to bind and the first one unreachable by it.
const CHANNEL_PORT = process.env.EINK_CHANNEL_PORT ?? '0'
const SERVER_URL = process.env.EINK_SERVER_URL ?? 'http://localhost:3333'

// Sessions subscribed by this Claude instance — only these receive channel events.
const subscribedSessions = new Set<string>()

// Feedback ids already delivered. Bounded: a long-lived instance would otherwise grow
// this set for every note the reader ever sends.
const deliveredFeedback = new Map<string, true>()
const DELIVERED_FEEDBACK_LIMIT = 512

function rememberDelivered(feedbackId: string) {
  deliveredFeedback.set(feedbackId, true)
  while (deliveredFeedback.size > DELIVERED_FEEDBACK_LIMIT) {
    const oldest = deliveredFeedback.keys().next().value
    if (oldest === undefined) break
    deliveredFeedback.delete(oldest)
  }
}

function compactEvent(data: Record<string, unknown>) {
  const compact = { ...data }
  if (Array.isArray(compact.annotations)) {
    compact.annotations = compact.annotations.map((annotation, index) => {
      const { strokes, pressures, ...note } = annotation as Record<string, unknown>
      return { ...note, index, stroke_count: Array.isArray(strokes) ? strokes.length : 0 }
    })
  }
  if (Array.isArray(compact.annotation_images)) {
    compact.annotation_image_count = compact.annotation_images.length
    delete compact.annotation_images
  }
  delete compact.stroke_data
  return compact
}

/**
 * Deliver one event to this instance's Claude session.
 *
 * Returns false when the session is not ours. That must reach the caller as a failure:
 * answering 200 for a session nobody here subscribed to tells the server the feedback
 * landed, and it is then dropped with the tablet still waiting.
 */
async function deliver(session_id: string, data: Record<string, unknown>): Promise<boolean> {
  if (!subscribedSessions.has(session_id)) {
    process.stderr.write(`eink-channel skip session=${session_id} (not subscribed on this instance)\n`)
    return false
  }
  const feedbackId = typeof data.feedback_id === 'string' ? data.feedback_id : null
  if (feedbackId && deliveredFeedback.has(feedbackId)) {
    process.stderr.write(`eink-channel duplicate session=${session_id} feedback=${feedbackId}\n`)
    return true
  }
  const event_type = data.type === 'annotation_result' ? 'annotation_result'
    : data.status === 'Submitted' ? 'submitted' : 'cancelled'
  await mcp.notification({
    method: 'notifications/claude/channel',
    params: { content: JSON.stringify(compactEvent(data)), meta: { event_type, session_id } },
  })
  process.stderr.write(
    `eink-channel deliver session=${session_id} event=${event_type} feedback=${feedbackId ?? '-'}\n`)
  if (feedbackId) rememberDelivered(feedbackId)
  if (event_type === 'submitted' || event_type === 'cancelled') subscribedSessions.delete(session_id)
  return true
}

const mcp = new Server(
  { name: 'eink-channel', version: '0.1.0' },
  {
    capabilities: {
      experimental: { 'claude/channel': {} },
      tools: {},
    },
    instructions:
      'Eink tablet events arrive as <channel source="eink-channel" event_type="..." session_id="...">. ' +
      'event_type values: annotation_result (user annotated — rewrite doc and call update_session), ' +
      'submitted (review complete — summarize feedback), ' +
      'cancelled (session cancelled). ' +
      'After creating a session with eink-review push, call subscribe_session immediately so events are routed to this instance. ' +
      'To push updated content back to the tablet, call the update_session tool. ' +
      'Pass feedback_id and expected_version when responding. consumed_annotations are zero-based annotation array indices, not bind-group IDs.',
  },
)

mcp.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
    {
      name: 'subscribe_session',
      description: 'Register this Claude instance as the owner of a session so that webhook events for it are routed here and not to other running instances. Call this immediately after eink-review push.',
      inputSchema: {
        type: 'object',
        properties: {
          session_id: { type: 'string', description: 'The session ID to claim' },
        },
        required: ['session_id'],
      },
    },
    {
      name: 'update_session',
      description: 'Push updated markdown. Acknowledge feedback_id with expected_version; consumed_annotations are zero-based array indices of incorporated notes, not bind-group IDs.',
      inputSchema: {
        type: 'object',
        properties: {
          session_id: { type: 'string', description: 'The session ID to update' },
          content: { type: 'string', description: 'Full markdown content to display' },
          consumed_annotations: {
            type: 'array',
            items: { type: 'integer' },
            description: 'Zero-based indices in the feedback annotations array that have been incorporated; requires feedback_id',
          },
          feedback_id: { type: 'string', description: 'ID of the feedback being acknowledged' },
          expected_version: { type: 'integer', description: 'Document version used for this rewrite; stale updates are rejected' },
        },
        required: ['session_id', 'content'],
      },
    },
    {
      name: 'report_progress',
      description: 'Tell the tablet what you are doing while you rewrite. One short line in the reader\'s words; it replaces "Waiting for agent" and is cleared by the update that names the same feedback_id.',
      inputSchema: {
        type: 'object',
        properties: {
          session_id: { type: 'string', description: 'The session being rewritten' },
          feedback_id: { type: 'string', description: 'The feedback this work answers' },
          message: { type: 'string', description: 'At most 120 characters, reader-facing' },
          step: { type: 'integer', description: 'Which step this is, 1-based (requires of)' },
          of: { type: 'integer', description: 'How many steps there are' },
        },
        required: ['session_id', 'feedback_id', 'message'],
      },
    },
  ],
}))

mcp.setRequestHandler(CallToolRequestSchema, async req => {
  if (req.params.name === 'subscribe_session') {
    const { session_id } = req.params.arguments as { session_id: string }
    await listening
    // Tell the server where this instance lives. `push --callback-url` cannot: the agent
    // does not know which channel process will claim the session, and after a restart the
    // stored URL points at a process that no longer exists. Registering on every claim is
    // what repairs that.
    const registered = await fetch(`${SERVER_URL}/api/sessions/${encodeURIComponent(session_id)}/callback`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: callbackUrl }),
    })
    if (!registered.ok && registered.status !== 404) {
      throw new Error(`subscribe_session could not register its callback: ${registered.status}`)
    }
    if (registered.status === 404) {
      process.stderr.write(
        'eink-channel warning: the server has no callback route; pass --callback-url to push\n')
    }
    subscribedSessions.add(session_id)
    process.stderr.write(`eink-channel subscribe session=${session_id} callback=${callbackUrl}\n`)
    // Callback may have arrived before the session was claimed, or while this
    // process was down. Recover the durable request, not only future webhooks.
    const response = await fetch(`${SERVER_URL}/api/sessions/${encodeURIComponent(session_id)}/feedback`)
    if (!response.ok) throw new Error(`subscribe_session failed: ${response.status}`)
    const snapshot = await response.json() as { pending?: Record<string, unknown> }
    if (snapshot.pending) await deliver(session_id, snapshot.pending)
    return { content: [{ type: 'text', text: `subscribed to session ${session_id} (callback ${callbackUrl})` }] }
  }
  if (req.params.name === 'update_session') {
    const { session_id, content, consumed_annotations, feedback_id, expected_version } = req.params.arguments as {
      session_id: string; content: string; consumed_annotations?: number[]; feedback_id?: string; expected_version?: number
    }
    const resp = await fetch(`${SERVER_URL}/api/sessions/${encodeURIComponent(session_id)}/content`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ content, consumed_annotations, feedback_id, expected_version }),
    })
    if (!resp.ok) {
      throw new Error(`update_session failed: ${resp.status} ${await resp.text()}`)
    }
    const data = await resp.json() as { version: number }
    return { content: [{ type: 'text', text: `updated to version ${data.version}` }] }
  }
  if (req.params.name === 'report_progress') {
    const { session_id, feedback_id, message, step, of } = req.params.arguments as {
      session_id: string; feedback_id: string; message: string; step?: number; of?: number
    }
    const resp = await fetch(
      `${SERVER_URL}/api/sessions/${encodeURIComponent(session_id)}/progress`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ feedback_id, message, step, of }),
      })
    if (!resp.ok) {
      throw new Error(`report_progress failed: ${resp.status} ${await resp.text()}`)
    }
    return { content: [{ type: 'text', text: `progress reported: ${message}` }] }
  }
  throw new Error(`unknown tool: ${req.params.name}`)
})

await mcp.connect(new StdioServerTransport())

let callbackUrl = ''

const httpServer = createServer((req, res) => {
  if (req.method !== 'POST') {
    res.writeHead(405).end('method not allowed')
    return
  }
  const chunks: Buffer[] = []
  req.on('data', chunk => chunks.push(chunk))
  req.on('end', async () => {
    const body = Buffer.concat(chunks).toString()
    let data: Record<string, unknown> = {}
    try {
      data = JSON.parse(body)
    } catch {
      res.writeHead(400).end('invalid JSON')
      return
    }

    const session_id = String(data.id ?? '')
    try {
      const delivered = await deliver(session_id, data)
      if (!delivered) {
        res.writeHead(404, { 'content-type': 'application/json' })
        res.end(JSON.stringify({ error: 'session is not subscribed to this channel instance' }))
        return
      }
      res.writeHead(200).end('ok')
    } catch {
      res.writeHead(503).end('delivery failed; feedback remains on server')
    }
  })
})

// Announce the chosen port before anything can subscribe. `listening` is awaited by
// subscribe_session, so a tool call cannot race the bind.
const listening = new Promise<void>((resolve, reject) => {
  httpServer.once('listening', () => {
    const address = httpServer.address()
    if (address === null || typeof address === 'string') {
      reject(new Error('the channel did not get a TCP port'))
      return
    }
    callbackUrl = `http://127.0.0.1:${address.port}`
    process.stderr.write(`eink-channel listening on ${callbackUrl}\n`)
    resolve()
  })
  httpServer.once('error', (error: Error) => {
    process.stderr.write(`eink-channel cannot listen on port ${CHANNEL_PORT}: ${error.message}\n`)
    reject(error)
  })
})
httpServer.listen(Number(CHANNEL_PORT), '127.0.0.1')
