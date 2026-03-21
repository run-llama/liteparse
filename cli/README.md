# cli/

Command-line interface for LiteParse using Commander.js.

## Files

### parse.ts
**CLI entry point with two main commands: `parse` and `screenshot`.**

---

## Commands

### `lit parse <file>`

Parse documents and extract text.

---

### `lit screenshot <file> -o <output_dir>`

Generate page screenshots.

---

## Configuration File

Both commands accept `--config <file>` to load settings from JSON:

```json
{
  "ocrEnabled": true,
  "ocrLanguage": "en",
  "ocrServerUrl": "http://localhost:5000/ocr",
  "maxPages": 100,
  "dpi": 200,
  "outputFormat": "json",
  "preciseBoundingBox": true,
  "password": "optional_password"
}
```

CLI options override config file values.

---

## Adding CLI Options

1. Add `.option()` call in the command definition
2. Read option in action handler from `options` object
3. Add to config object that's passed to `LiteParse`
4. If new config field, update `src/core/types.ts` and `src/core/config.ts`
