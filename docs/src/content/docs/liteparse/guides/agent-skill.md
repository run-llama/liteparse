---
title: Agent Skill
description: Add LiteParse as a skill for coding agents like Claude Code, Cursor, and others.
sidebar:
  order: 6
---

LiteParse can be installed as a **coding agent skill** using Vercel's [skills](https://github.com/vercel-labs/skills) utility. This gives your coding agent the ability to process documents, generate screenshots, and parse text from files, all locally.

## Installation

Add the LiteParse skill to your project:

```bash
npx skills add run-llama/llamaparse-agent-skills --skill liteparse
```

This downloads a skill file that compatible coding agents (Claude Code, Cursor, etc.) will automatically pick up.

Once configured, your agent will be able to call the LiteParse CLI commands directly from its code execution environment. This means you can have your agent parse PDFs, pull out the text, and generate screenshots on the fly as part of its reasoning process.

## Example prompts

Once the skill is installed, you can ask your coding agent things like:

- "Parse this PDF and extract the text as JSON"
- "Extract text from all the DOCX files in the `./contracts` folder"
- "Screenshot pages 1-5 of this PDF at 300 DPI"
- "Parse this scanned document using the PaddleOCR server on localhost:8828"
- "Get the bounding boxes for all text on page 3"


## Configuring Defaults

You might want to configure some defaults so that your agent doesn't have to specify them in every prompt. You can create a `liteparse.config.json` file in the root of your project with settings like:

```json
{
  "ocrLanguage": "en",
  "ocrEnabled": true,
  "maxPages": 1000,
  "dpi": 150,
  "outputFormat": "json",
  "preserveVerySmallText": false
}
```

This is especially useful for custom OCR servers. Just add the `ocrServerUrl` to your config:

```json
{
  "ocrServerUrl": "http://localhost:8828/ocr",
  "ocrLanguage": "en",
  "outputFormat": "json"
}
```
