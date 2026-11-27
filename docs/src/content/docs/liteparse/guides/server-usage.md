---
title: Server Usage
description: Run LiteParse as an HTTP service with liteparse-server.
sidebar:
  order: 11
---

[`liteparse-server`](https://github.com/run-llama/liteparse-server) is an Express server that exposes [`@llamaindex/liteparse`](https://www.npmjs.com/package/@llamaindex/liteparse) as an HTTP parsing backend. It lets you run LiteParse as a standalone service, which is useful when your application isn't Node.js, when you want to share one parsing instance across multiple clients, or when you need a parsing endpoint behind your own infrastructure.

The server ships in two flavours:

- **Slim** — a minimal build with no external dependencies, ideal for getting started or embedding in your own stack.
- **Full** — an all-in-one setup with built-in **Redis caching and rate limiting**, **OpenTelemetry tracing** (Jaeger), and **metrics** (Prometheus + Grafana), wired together with Docker Compose.

Both expose the same two endpoints: `POST /parse` and `POST /screenshots`.

## Requirements

- [Bun](https://bun.sh) ≥ 1.0 (or Node.js)
- [Docker](https://docs.docker.com/get-docker/) and Docker Compose (only required for the full server setup)

## Slim server

[`src/slim.ts`](https://github.com/run-llama/liteparse-server/tree/main/src/slim.ts) is a minimal version of the server that removes caching and observability while keeping rate limiting and logging. It needs no Redis, no OpenTelemetry collector, and no supporting services.

### Running locally

After cloning the GitHub repository:

```bash
git clone https://github.com/run-llama/liteparse-server
cd liteparse-server
```

You can run the API server with either Bun or Node:

```bash
bun run start-slim:bun
# or with Node
npm run start-slim:node
```

The server listens on **port 5000**.

### Running with Docker

**Pre-built image**

You can pull the pre-built LiteParse (slim) server image from the GitHub Container Registry:

```bash
docker pull ghcr.io/run-llama/liteparse-server:main
```

You can then run it exposing port 5000:

```bash
docker run -p 5000:5000 ghcr.io/run-llama/liteparse-server:main
```

**Build locally**

If you clone the repository, the provided [`slim.Dockerfile`](https://github.com/run-llama/liteparse-server/tree/main/slim.Dockerfile) produces a self-contained image:

```bash
# Build the image
docker build -f slim.Dockerfile -t liteparse-server-slim .

# Run exposing port 5000
docker run -p 5000:5000 liteparse-server-slim
```

The API is then available at **http://localhost:5000**.

## Full server

For a production-style deployment with caching, rate limiting, distributed tracing, and metrics, follow the Docker Compose example in [`examples/docker-compose`](https://github.com/run-llama/liteparse-server/tree/main/examples/docker-compose). It brings up the server alongside Redis, the OpenTelemetry Collector, Jaeger, Prometheus, and Grafana.

## API specification

Base URL: `http://localhost:5000`

### `POST /parse` — parse a single file

Parses a single document and returns either structured page data or plain text.

**Form fields:**

| Field    | Type   | Required | Description                               |
| -------- | ------ | -------- | ----------------------------------------- |
| `file`   | file   | Yes      | The document to parse                     |
| `config` | string | No       | JSON-serialized `LiteParseConfig` options |

**Query parameters:**

| Parameter | Type    | Default | Description                                                                        |
| --------- | ------- | ------- | ---------------------------------------------------------------------------------- |
| `text`    | boolean | `false` | If `true`, returns `text/plain`; otherwise `application/json` with a `pages` array |

**Responses:**

- `200 text/plain` — extracted text (when `text=true`)
- `200 application/json` — `{ "pages": [...] }` (when `text=false`)
- `400` — missing `file`
- `429` — rate limit exceeded (only if rate limiting is configured)

### `POST /screenshots` — screenshot pages of a document

Renders document pages as PNG images and streams them back as newline-delimited JSON (NDJSON).

**Form fields:**

| Field    | Type   | Required | Description                               |
| -------- | ------ | -------- | ----------------------------------------- |
| `file`   | file   | Yes      | The document to screenshot                |
| `config` | string | No       | JSON-serialized `LiteParseConfig` options |

**Query parameters:**

| Parameter | Type   | Default | Description                                                       |
| --------- | ------ | ------- | ----------------------------------------------------------------- |
| `pages`   | string | all     | Comma-separated 1-based page numbers to screenshot (e.g. `1,2,3`) |

**Response `200 application/x-ndjson`** — one JSON object per line:

```json
{
  "index": 0,
  "mimetype": "image/png",
  "data": "<base64>",
  "page_number": 1,
  "height": 1056,
  "width": 816
}
```

## Example usage with `curl`

Parse a file and get JSON pages:

```bash
curl -X POST http://localhost:5000/parse \
  -F "file=@path/to/document.pdf"
```

Parse a file and get plain text:

```bash
curl -X POST "http://localhost:5000/parse?text=true" \
  -F "file=@path/to/document.pdf"
```

Pass a `LiteParseConfig` (e.g. enable OCR):

```bash
curl -X POST http://localhost:5000/parse \
  -F "file=@path/to/document.pdf" \
  -F 'config={"ocrEnabled":true}'
```

Screenshot specific pages and stream NDJSON:

```bash
curl -X POST "http://localhost:5000/screenshots?pages=1,2,3" \
  -F "file=@path/to/document.pdf"
```
