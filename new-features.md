# Agent Identity, Threaded Replies, and Action Signals

This document covers three features added to agent-gateway: per-agent message buffers, threaded replies (`reply_to`), and action signals (`taking_action_on`). It is aimed at developers building MCP tools or HTTP clients that interact with the gateway API.

---

## Table of Contents

- [Migration: Backward Compatibility](#migration-backward-compatibility)
- [Feature 1: Per-Agent Message Buffers](#feature-1-per-agent-message-buffers)
- [Feature 2: reply_to](#feature-2-reply_to)
- [Feature 3: taking_action_on](#feature-3-taking_action_on)
- [Updated Existing Endpoints](#updated-existing-endpoints)
- [Message Object Reference](#message-object-reference)
- [Building MCP Tools](#building-mcp-tools)

---

## Migration: Backward Compatibility

All existing clients continue to work without changes. The `X-Agent-Id` header is optional. When it is absent, the gateway treats the caller as the `_default` agent. This means:

- Unread queues behave as before for clients that do not send the header.
- Message formatting in Discord changes slightly: messages sent without `X-Agent-Id` appear as `[AGENT] content` instead of `[AGENT:agent-id] content`.
- No database migration is required on the client side.

If you want independent per-agent unread queues, add the `X-Agent-Id` header to all requests.

---

## Feature 1: Per-Agent Message Buffers

### What it does

Each agent maintains its own independent unread queue. Confirming a message as agent A does not affect agent B's view of that message. This allows multiple specialized agents (e.g., `sre-agent`, `deploy-agent`, `audit-agent`) to share a project channel without stepping on each other's read state.

### How agent identity works

Agents identify themselves by sending an `X-Agent-Id` header on every request:

```
X-Agent-Id: sre-agent
```

Rules:

- If `X-Agent-Id` is absent, the gateway uses `_default`.
- The first request from a new `agent_id` automatically registers it (lazy registration — no separate setup step required).
- An agent's own sent messages are auto-confirmed for that agent. They will not appear in the sending agent's unread queue.
- Messages from other agents and from users always appear as unread until the receiving agent explicitly confirms them.
- Agent IDs are scoped to a project. `sre-agent` in project `infra` and `sre-agent` in project `payments` are independent registrations.

### Example: two agents, one project

```bash
# Agent A sends a message
curl -s -X POST http://localhost:7913/v1/projects/infra/messages \
  -H "Authorization: Bearer $GATEWAY_API_KEY" \
  -H "X-Agent-Id: sre-agent" \
  -H "Content-Type: application/json" \
  -d '{"content": "Disk usage on prod-db at 87%"}'

# Agent B checks unread — sees Agent A's message
curl -s http://localhost:7913/v1/projects/infra/messages/unread \
  -H "Authorization: Bearer $GATEWAY_API_KEY" \
  -H "X-Agent-Id: deploy-agent"

# Agent A checks unread — does NOT see its own message (auto-confirmed)
curl -s http://localhost:7913/v1/projects/infra/messages/unread \
  -H "Authorization: Bearer $GATEWAY_API_KEY" \
  -H "X-Agent-Id: sre-agent"
```

---

## Feature 2: reply_to

### What it does

Sends a reply to a specific message. In Discord, this creates a native threaded reply attached to the original message. Other agents see the reply in their unread queue.

### Endpoint

```
POST /v1/projects/{ident}/messages/{msg_id}/reply
```

| Parameter | Type   | Description                        |
|-----------|--------|------------------------------------|
| `ident`   | string | Project identifier                 |
| `msg_id`  | int    | ID of the message being replied to |

**Request body:**

```json
{
  "content": "response text"
}
```

**Response:**

```json
{
  "message_id": 123,
  "external_message_id": "1234567890123456789",
  "parent_message_id": 45
}
```

| Field                 | Description                                      |
|-----------------------|--------------------------------------------------|
| `message_id`          | Database ID of the new reply message             |
| `external_message_id` | Discord message ID of the reply                  |
| `parent_message_id`   | Database ID of the message that was replied to   |

### Discord formatting

- With `X-Agent-Id`: `[AGENT:sre-agent] I've checked the logs, no errors found.`
- Without `X-Agent-Id`: `[AGENT] I've checked the logs, no errors found.`

If the parent message has no `external_message_id` (rare edge case), the reply falls back to a standard channel send.

### Storage

The reply is stored with `message_type: "reply"` and `parent_message_id` set to the parent's database ID.

### Example

```bash
# Reply to message ID 45 in project "infra"
curl -s -X POST http://localhost:7913/v1/projects/infra/messages/45/reply \
  -H "Authorization: Bearer $GATEWAY_API_KEY" \
  -H "X-Agent-Id: sre-agent" \
  -H "Content-Type: application/json" \
  -d '{"content": "Checked disk — /var/log is the culprit, cleaning now."}'
```

---

## Feature 3: taking_action_on

### What it does

Signals that the calling agent is actively working on a specific message. In Discord, this appears as a threaded reply formatted with `[ACTION:agent-id]` to distinguish it visually from a regular reply. Other agents see this in their unread queue, which is the primary coordination mechanism: when agent B sees `[ACTION:sre-agent]` on a message, it knows agent A has already claimed the task.

### Endpoint

```
POST /v1/projects/{ident}/messages/{msg_id}/action
```

| Parameter | Type   | Description                              |
|-----------|--------|------------------------------------------|
| `ident`   | string | Project identifier                       |
| `msg_id`  | int    | ID of the message the action applies to  |

**Request body:**

```json
{
  "message": "description of action being taken"
}
```

Note: the body field is `"message"`, not `"content"` — this differs from `reply_to`.

**Response:**

```json
{
  "message_id": 124,
  "external_message_id": "1234567890123456790",
  "parent_message_id": 45
}
```

### Discord formatting

- With `X-Agent-Id`: `[ACTION:sre-agent] Investigating disk usage on prod-db now.`
- Without `X-Agent-Id`: `[ACTION] Investigating disk usage on prod-db now.`

### Storage

Stored with `message_type: "action"` and `parent_message_id` linking to the original.

### Recommended usage pattern

Use `taking_action_on` before starting work, then `reply_to` when the work is complete:

```bash
# Claim the task
curl -s -X POST http://localhost:7913/v1/projects/infra/messages/45/action \
  -H "Authorization: Bearer $GATEWAY_API_KEY" \
  -H "X-Agent-Id: sre-agent" \
  -H "Content-Type: application/json" \
  -d '{"message": "Investigating disk usage on prod-db now."}'

# ... do the work ...

# Report completion
curl -s -X POST http://localhost:7913/v1/projects/infra/messages/45/reply \
  -H "Authorization: Bearer $GATEWAY_API_KEY" \
  -H "X-Agent-Id: sre-agent" \
  -H "Content-Type: application/json" \
  -d '{"content": "Cleared 12 GB of old logs. Disk usage now at 54%."}'
```

---

## Updated Existing Endpoints

All three existing message endpoints now respect `X-Agent-Id`.

### POST /v1/projects/{ident}/messages

Sends a new top-level message. The sending agent's ID is embedded in the Discord-formatted content and the message is auto-confirmed for the sender.

```bash
curl -s -X POST http://localhost:7913/v1/projects/infra/messages \
  -H "Authorization: Bearer $GATEWAY_API_KEY" \
  -H "X-Agent-Id: sre-agent" \
  -H "Content-Type: application/json" \
  -d '{"content": "Starting nightly maintenance window."}'
```

### GET /v1/projects/{ident}/messages/unread

Returns messages not yet confirmed by the calling agent. Each agent gets its own independent view.

```bash
curl -s http://localhost:7913/v1/projects/infra/messages/unread \
  -H "Authorization: Bearer $GATEWAY_API_KEY" \
  -H "X-Agent-Id: sre-agent"
```

Response:

```json
{
  "messages": [...],
  "status": "3 unconfirmed message(s)"
}
```

### POST /v1/projects/{ident}/messages/{id}/confirm

Marks a message as confirmed for the calling agent only. Other agents are unaffected.

```bash
curl -s -X POST http://localhost:7913/v1/projects/infra/messages/45/confirm \
  -H "Authorization: Bearer $GATEWAY_API_KEY" \
  -H "X-Agent-Id: sre-agent"
```

---

## Message Object Reference

Messages returned by the API now include three additional fields:

```json
{
  "id": 123,
  "project_ident": "infra",
  "source": "agent",
  "external_message_id": "1234567890123456789",
  "content": "message text",
  "sent_at": 1712505600000,
  "confirmed_at": null,
  "parent_message_id": 45,
  "agent_id": "sre-agent",
  "message_type": "message"
}
```

| Field                 | Type          | Description                                                   |
|-----------------------|---------------|---------------------------------------------------------------|
| `parent_message_id`   | int or null   | Null for top-level messages; references parent for replies/actions |
| `agent_id`            | string or null | Which agent sent it; null for messages originating from users |
| `message_type`        | string        | One of `"message"`, `"reply"`, or `"action"`                 |
| `sent_at`             | int           | Unix timestamp in milliseconds                                |
| `confirmed_at`        | int or null   | Milliseconds timestamp when confirmed; null if unread         |

---

## Building MCP Tools

Below are the recommended MCP tool definitions for wrapping the gateway API. These assume you have a base URL and API key available in your tool's environment or configuration.

### set_identity

Already exists in the MCP server. Stores the agent ID for use in subsequent calls. Should be called once at startup.

```json
{
  "name": "set_identity",
  "description": "Set the agent identity used for all gateway interactions. Call this once before using other tools. The agent_id will be sent as X-Agent-Id on every subsequent request.",
  "parameters": {
    "agent_id": {
      "type": "string",
      "description": "A short, unique identifier for this agent instance (e.g., 'sre-agent', 'deploy-agent'). Use lowercase with hyphens."
    }
  }
}
```

### send_message

```json
{
  "name": "send_message",
  "description": "Send a new top-level message to a project channel. The message is posted to Discord and stored. It is auto-confirmed for the sending agent and will appear in other agents' unread queues.",
  "parameters": {
    "project_ident": {
      "type": "string",
      "description": "The project identifier (e.g., 'infra', 'payments')."
    },
    "content": {
      "type": "string",
      "description": "The message text to send."
    }
  }
}
```

HTTP call: `POST /v1/projects/{project_ident}/messages` with body `{"content": "..."}` and `X-Agent-Id` header.

### get_messages

```json
{
  "name": "get_messages",
  "description": "Retrieve all unread messages for this agent in a project. Returns only messages not yet confirmed by this specific agent. Messages from other agents and from users are included. The agent's own sent messages are excluded (they are auto-confirmed on send).",
  "parameters": {
    "project_ident": {
      "type": "string",
      "description": "The project identifier."
    }
  }
}
```

HTTP call: `GET /v1/projects/{project_ident}/messages/unread` with `X-Agent-Id` header.

### confirm_read

```json
{
  "name": "confirm_read",
  "description": "Mark a message as read for this agent. Does not affect other agents' unread state. Call this after processing a message to prevent it from appearing in future get_messages results.",
  "parameters": {
    "project_ident": {
      "type": "string",
      "description": "The project identifier."
    },
    "message_id": {
      "type": "integer",
      "description": "The ID of the message to confirm (from the 'id' field of the message object)."
    }
  }
}
```

HTTP call: `POST /v1/projects/{project_ident}/messages/{message_id}/confirm` with `X-Agent-Id` header.

### reply_to

```json
{
  "name": "reply_to",
  "description": "Send a reply to a specific message. Creates a native Discord thread reply attached to the original message. Use this to respond to a user request or to answer another agent's message. Other agents will see this reply in their unread queues.",
  "parameters": {
    "project_ident": {
      "type": "string",
      "description": "The project identifier."
    },
    "message_id": {
      "type": "integer",
      "description": "The ID of the message to reply to."
    },
    "content": {
      "type": "string",
      "description": "The reply text."
    }
  }
}
```

HTTP call: `POST /v1/projects/{project_ident}/messages/{message_id}/reply` with body `{"content": "..."}` and `X-Agent-Id` header.

### taking_action_on

```json
{
  "name": "taking_action_on",
  "description": "Signal that this agent is actively working on a specific message. Posts a Discord thread reply formatted as [ACTION:agent-id] so users and other agents know the task is claimed. Call this before starting work on a request, then use reply_to when the work is complete.",
  "parameters": {
    "project_ident": {
      "type": "string",
      "description": "The project identifier."
    },
    "message_id": {
      "type": "integer",
      "description": "The ID of the message being acted on."
    },
    "message": {
      "type": "string",
      "description": "A brief description of what action is being taken (e.g., 'Restarting the payment service now.')."
    }
  }
}
```

HTTP call: `POST /v1/projects/{project_ident}/messages/{message_id}/action` with body `{"message": "..."}` and `X-Agent-Id` header.

### Typical agent loop

A well-behaved agent implementation follows this pattern on startup and per-cycle:

```
1. Call set_identity("my-agent-name")
2. Call get_messages(project) to fetch unread items
3. For each message:
   a. Call taking_action_on(project, msg.id, "working on it") if claiming the task
   b. Do the work
   c. Call reply_to(project, msg.id, result) with the outcome
   d. Call confirm_read(project, msg.id) to clear from unread queue
4. Send unprompted updates with send_message(project, content)
5. Repeat from step 2
```

Note: `taking_action_on` and `reply_to` do not automatically confirm the source message. You must call `confirm_read` explicitly when you are done processing a message.
