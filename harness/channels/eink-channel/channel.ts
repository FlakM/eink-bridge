#!/usr/bin/env node --experimental-strip-types
import { createServer } from 'node:http'
import { Server } from '@modelcontextprotocol/sdk/server/index.js'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { ListToolsRequestSchema, CallToolRequestSchema } from '@modelcontextprotocol/sdk/types.js'

const CHANNEL_PORT = parseInt(process.env.EINK_CHANNEL_PORT ?? '8789', 10)
const SERVER_URL = process.env.EINK_SERVER_URL ?? 'http://localhost:3333'

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
      'Match session_id to know which session the event belongs to. ' +
      'To push updated content back to the tablet, call the update_session tool.',
  },
)

mcp.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
    {
      name: 'update_session',
      description: 'Push updated markdown content to an active eink session on the tablet',
      inputSchema: {
        type: 'object',
        properties: {
          session_id: { type: 'string', description: 'The session ID to update' },
          content: { type: 'string', description: 'Full markdown content to display' },
        },
        required: ['session_id', 'content'],
      },
    },
  ],
}))

mcp.setRequestHandler(CallToolRequestSchema, async req => {
  if (req.params.name === 'update_session') {
    const { session_id, content } = req.params.arguments as { session_id: string; content: string }
    const resp = await fetch(`${SERVER_URL}/api/sessions/${session_id}/content`, {
      method: 'PUT',
      headers: { 'Content-Type': 'text/plain' },
      body: content,
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

    await mcp.notification({
      method: 'notifications/claude/channel',
      params: { content: body, meta: { event_type, session_id } },
    })
    res.writeHead(200).end('ok')
  })
})

httpServer.listen(CHANNEL_PORT, '127.0.0.1')
