# Stage 1: Build from source (needs build tools for native addons like sharp, pdfium)
FROM node:24-trixie AS builder

WORKDIR /build
COPY package.json package-lock.json ./
RUN npm ci --ignore-scripts=false

COPY src/ ./src/
COPY cli/ ./cli/
COPY tsconfig.json ./
RUN npm run build

# Stage 2: Minimal runtime image
FROM node:24-trixie-slim

# sharp and @hyzyla/pdfium ship prebuilt binaries but may need these shared libs at runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    libvips42 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/local/lib/liteparse
COPY --from=builder /build/package.json ./
COPY --from=builder /build/dist/ ./dist/
COPY --from=builder /build/src/vendor/pdfjs ./src/vendor/pdfjs/
COPY --from=builder /build/node_modules/ ./node_modules/

# Make the CLI executable + globally available via symlinks
RUN chmod +x /usr/local/lib/liteparse/dist/src/index.js
RUN ln -s /usr/local/lib/liteparse/dist/src/index.js /usr/local/bin/lit \
    && ln -s /usr/local/lib/liteparse/dist/src/index.js /usr/local/bin/liteparse


CMD ["/bin/sh"]
