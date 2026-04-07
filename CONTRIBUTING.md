# Contributing to LiteParse

Thank you for your interest in contributing to LiteParse! This document provides guidelines and information for contributors.

## Getting Started

1. Fork the repository
2. Clone your fork:
   ```bash
   git clone https://github.com/YOUR_USERNAME/liteparse.git
   cd liteparse
   ```
3. Install dependencies:
   ```bash
   npm install
   ```
4. Build the project:
   ```bash
   npm run build
   ```

## What to Contribute?

In this project, we welcome a wide range of contributions, but we do want to maintain the spirit of the project. We are primarily focused on:

- Core algorithms for PDF parsing and text extraction
- OCR integrations and improvements
- Different types or modifications to output formats

We are less interested in:

- Markdown output
- Any LLM integration or agent code
- Anything that doesn't directly relate to improving the core parsing and extraction capabilities

While the project is in Typescript today, I'm pretty open to porting to Rust if someone wanted to take that on as a contribution. The core algorithms and logic would be the same, just implemented in Rust instead of Typescript.

## Development Workflow

### Building

```bash
npm run build      # Build TypeScript
npm run dev        # Watch mode for development
```

### Testing

```bash
npm test           # Run tests
npm run test:watch # Run tests in watch mode
```

### Linting & Formatting

```bash
npm run lint       # Check for linting issues
npm run lint:fix   # Fix linting issues
npm run format     # Format code with Prettier
```

### Testing Local Changes

You can test your changes locally:

```bash
# Parse a document
./dist/src/index.js parse document.pdf

# Generate screenshots
./dist/src/index.js screenshot document.pdf -o ./screenshots
```

### Debugging Grid Projection

When working on the grid projection algorithm (`src/processing/gridProjection.ts`), you can enable built-in debug logging and visual output instead of adding ad-hoc `console.log` statements.

**Debug logging** traces every decision the projection makes — block detection, anchor extraction, snap assignment, rendering, and flowing text classification:

```bash
# Log all projection decisions to stderr
./dist/src/index.js parse document.pdf --debug

# Filter to a specific page
./dist/src/index.js parse document.pdf --debug --debug-page 3

# Filter to elements containing specific text
./dist/src/index.js parse document.pdf --debug --debug-text-filter "Total" "Revenue"

# Filter to a bounding region (x1,y1,x2,y2 in PDF points)
./dist/src/index.js parse document.pdf --debug --debug-region "0,100,300,200"

# Write debug log to a file
./dist/src/index.js parse document.pdf --debug --debug-output ./debug-output
```

**Visual grid export** generates PNG images showing text boxes color-coded by snap type (blue=left, red=right, green=center, gray=floating, yellow=flowing) with anchor lines overlaid. This is useful for comparing against page screenshots to spot projection issues:

```bash
# Generate visualization PNGs (one per page)
./dist/src/index.js parse document.pdf --debug-visualize

# Specify output directory
./dist/src/index.js parse document.pdf --debug-visualize --debug-output ./my-debug
```

These options are also available via the library API:

```typescript
const parser = new LiteParse({
  debug: {
    enabled: true,
    textFilter: ["Total"],
    pageFilter: 2,
    visualize: true,
    visualizePath: "./debug-output",
  }
});
```

See `src/processing/gridDebugLogger.ts` for the full `GridDebugConfig` interface and `src/processing/gridVisualizer.ts` for the visualization renderer.

## Making Changes

### Versioning & Changelogs

We use [Changesets](https://github.com/changesets/changesets) to manage versioning and changelogs. When you make a change to source code that should be released:

1. Run `npm run changeset`
2. Select the type of change (patch, minor, major)
3. Write a description of your changes
4. Commit the generated changeset file with your PR

## Pull Requests

1. Fork and create a feature branch from `main`
2. Make your changes
3. Add a changeset if needed (`npm run changeset`)
4. Ensure all tests pass (`npm test`)
5. Ensure linting passes (`npm run lint:fix` and `npm run format`)
6. Submit a pull request

When you submit a PR, a number of CICD checks will run. Among these, your code will be tested against a regression suite of documents to ensure that your changes don't break existing parsing capabilities. It will be up to the maintainers discretion to determine if any changes to the regression set are expected/positive or unexpected/negative.

### PR Guidelines

- Keep PRs focused on a single change
- Update documentation if needed
- Add tests for new functionality
- For parsing issues, include a test document if possible

## Reporting Issues

### Parsing Issues

If you're reporting a problem with document parsing:

1. **You must attach the document** or provide a way to reproduce the issue
2. Include the command you ran
3. Show the expected vs actual output
4. Include your LiteParse version (`lit --version`)

Issues without reproducible examples will be closed.

### Bug Reports

For other bugs:
1. Describe what you expected vs what happened
2. Include steps to reproduce
3. Include error messages/stack traces
4. Include version information

## Project Structure

See [AGENTS.md](AGENTS.md) for detailed documentation about the codebase structure and architecture.

Key directories:
- `src/core/` - Main orchestrator and configuration
- `src/engines/` - PDF and OCR engine implementations
- `src/processing/` - Text extraction and spatial analysis
- `src/output/` - Output formatters
- `cli/` - CLI implementation

## Questions?

- Open a [Discussion](https://github.com/run-llama/liteparse/discussions) for questions
- Check existing issues before opening new ones
- Read the [README](README.md) for usage documentation

## License

By contributing, you agree that your contributions will be licensed under the Apache 2.0 License.
