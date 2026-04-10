#!/usr/bin/env node --experimental-strip-types
import { createServer } from 'node:http'
import { Server } from '@modelcontextprotocol/sdk/server/index.js'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { ListToolsRequestSchema, CallToolRequestSchema } from '@modelcontextprotocol/sdk/types.js'

const CHANNEL_PORT = parseInt(process.env.EINK_CHANNEL_PORT ?? '8789', 10)
const SERVER_URL = process.env.EINK_SERVER_URL ?? 'http://localhost:3333'

// Sessions subscribed by this Claude instance — only these receive channel events.
const subscribedSessions = new Set<string>()

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
      'Pass consumed_annotations with the bind-group IDs from the annotation_result to clear those strokes from the tablet.',
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
      description: 'Push updated markdown content to an active eink session on the tablet. Pass consumed_annotations with the bind-group IDs you have understood so the tablet clears those strokes automatically.',
      inputSchema: {
        type: 'object',
        properties: {
          session_id: { type: 'string', description: 'The session ID to update' },
          content: { type: 'string', description: 'Full markdown content to display' },
          consumed_annotations: {
            type: 'array',
            items: { type: 'integer' },
            description: 'Bind-group IDs that have been incorporated into the document — the tablet removes these strokes automatically',
          },
        },
        required: ['session_id', 'content'],
      },
    },
  ],
}))

mcp.setRequestHandler(CallToolRequestSchema, async req => {
  if (req.params.name === 'subscribe_session') {
    const { session_id } = req.params.arguments as { session_id: string }
    subscribedSessions.add(session_id)
    return { content: [{ type: 'text', text: `subscribed to session ${session_id}` }] }
  }
  if (req.params.name === 'update_session') {
    const { session_id, content, consumed_annotations } = req.params.arguments as {
      session_id: string; content: string; consumed_annotations?: number[]
    }
    const hasConsumed = consumed_annotations && consumed_annotations.length > 0
    const resp = await fetch(`${SERVER_URL}/api/sessions/${session_id}/content`, {
      method: 'PUT',
      headers: { 'Content-Type': hasConsumed ? 'application/json' : 'text/plain' },
      body: hasConsumed ? JSON.stringify({ content, consumed_annotations }) : content,
    })
    if (!resp.ok) {
      throw new Error(`update_session failed: ${resp.status} ${await resp.text()}`)
    }
    const data = await resp.json() as { version: number }
    return { content: [{ type: 'text', text: `updated to version ${data.version}` }] }
  }
  throw new Error(`unknown tool: ${req.params.name}`)
})

await mcp.connect(new StdioServerTransport())

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

    const event_type =
      data.type === 'annotation_result' ? 'annotation_result'
      : data.status === 'Submitted'     ? 'submitted'
      : 'cancelled'
    const session_id = String(data.id ?? '')

    if (subscribedSessions.has(session_id)) {
      if (event_type === 'submitted' || event_type === 'cancelled') {
        subscribedSessions.delete(session_id)
      }
      await mcp.notification({
        method: 'notifications/claude/channel',
        params: { content: body, meta: { event_type, session_id } },
      })
    }
    res.writeHead(200).end('ok')
  })
})

httpServer.listen(CHANNEL_PORT, '127.0.0.1')
