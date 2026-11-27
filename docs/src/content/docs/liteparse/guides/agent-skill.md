---
title: Agent Skill
description: Add LiteParse as a skill for coding agents like Claude Code, Cursor, and others.
sidebar:
  order: 12
---

LiteParse ships as a **coding agent skill** — a self-contained capability that Claude Code, Cursor,
Codex, and other compatible agents load on demand. Installing it gives your agent the ability to
parse documents, extract text and bounding boxes, and generate screenshots, all locally and with no
API key.

## Installation

Add the LiteParse skill to your project with the [`skills`](https://github.com/vercel-labs/skills)
CLI:

```bash
npx skills add run-llama/llamaparse-agent-skills --skill liteparse
```

This downloads a skill file that compatible coding agents pick up automatically.

You can also copy [`SKILL.md`](https://github.com/run-llama/llamaparse-agent-skills/blob/main/skills/liteparse/SKILL.md)
into your own skills setup by hand, or install the `liteparse` plugin from the
[agent plugins marketplace](https://github.com/run-llama/llamaparse-agent-plugins) to enable it from
inside Claude Code or Codex.

### Requirements

| Requirement | Needed for |
| --- | --- |
| Node 18+ and `npm i -g @llamaindex/liteparse` | The `lit` CLI the skill drives — verify with `lit --version` |
| [LibreOffice](https://www.libreoffice.org/) | Parsing Office files (DOCX, XLSX, PPTX) |
| [`uv`](https://docs.astral.sh/uv/) | The bundled ranked-search helper |

No API key is required since everything runs on your machine.

## What the skill teaches

The skill is more than an install: it encodes the extraction patterns that keep an agent's context
small and its runs cheap. Without it, agents commonly re-parse the same document on every search and
dump entire pages into the conversation.

- **Parse once to a file, then search that file.** Each `lit parse` re-extracts the whole document,
  so the skill has the agent parse to a temp file and run all subsequent searches against it.
- **Minimize round-trips.** Fetch a match and its surrounding context in a single command, and batch
  independent lookups together rather than spending a turn per search term.
- **Bound every output.** Results are capped so a single lookup can't flood the context window.
- **Escalate to ranked search** when keyword matching stalls, instead of firing off keyword variants
  one turn at a time.

## Example prompts

Once the skill is installed, you can ask your coding agent things like:

- "Parse this PDF and extract the text as JSON"
- "Extract text from all the DOCX files in the `./contracts` folder"
- "Screenshot pages 1-5 of this PDF at 300 DPI"
- "Parse this scanned document using the PaddleOCR server on localhost:8828"
- "Get the bounding boxes for all text on page 3"

## Related

LiteParse is one of several ways to give an agent document-processing capabilities. For cloud parsing
of complex documents, MCP servers, and workflow nodes, see
[Using LlamaIndex with AI Agents](/for-agents/).
